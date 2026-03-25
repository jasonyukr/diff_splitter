use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "diff_splitter_test_{}_{}",
            std::process::id(),
            unique
        ));
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn skip_header_keeps_binary_entries_out_of_following_text_file() {
    let temp_dir = TempDir::new();
    let diff = concat!(
        "diff --git a/jdk/make/closed/tools/crypto/jce/sunjce_provider.jar b/jdk/make/closed/tools/crypto/jce/sunjce_provider.jar\n",
        "index 7a97a50332d2..35d89c6ce890 100644\n",
        "Binary files a/jdk/make/closed/tools/crypto/jce/sunjce_provider.jar and b/jdk/make/closed/tools/crypto/jce/sunjce_provider.jar differ\n",
        "diff --git a/jdk/make/closed/tools/source-bundles/exclude-all b/jdk/make/closed/tools/source-bundles/exclude-all\n",
        "index 4a3a022df995..6bfd6c58ecd9 100644\n",
        "--- a/jdk/make/closed/tools/source-bundles/exclude-all\n",
        "+++ b/jdk/make/closed/tools/source-bundles/exclude-all\n",
        "@@ -137,11 +137,62 @@ hotspot/test/closed\n",
        " #\n",
        " # Embedded jdk files\n",
        " #\n",
        "+jdk/src/share/classes/org/openjdk\n",
    );

    let mut child = Command::new(env!("CARGO_BIN_EXE_diff_splitter"))
        .arg("--mask-linenum")
        .arg("--skip-header")
        .arg(temp_dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(diff.as_bytes())
        .unwrap();

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let binary_file_path = temp_dir.path().join("__BINARY_FILES__.txt");
    assert_eq!(
        fs::read_to_string(&binary_file_path).unwrap(),
        "Binary files a/jdk/make/closed/tools/crypto/jce/sunjce_provider.jar and b/jdk/make/closed/tools/crypto/jce/sunjce_provider.jar differ\n"
    );

    let output_file = temp_dir
        .path()
        .join("jdk/make/closed/tools/source-bundles/exclude-all");
    let output_contents = fs::read_to_string(output_file).unwrap();

    assert_eq!(
        output_contents,
        concat!(
            "@@ -000,11 +000,62 @@ hotspot/test/closed\n",
            " #\n",
            " # Embedded jdk files\n",
            " #\n",
            "+jdk/src/share/classes/org/openjdk\n",
        )
    );
    assert!(!output_contents.contains("Binary files "));
    assert!(!output_contents.contains("diff --git "));
    assert!(!output_contents.contains("\nindex "));
}
