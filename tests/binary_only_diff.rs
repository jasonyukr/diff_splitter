mod common;

use common::{list_output_files, read_output, read_output_bytes, run_diff_splitter};

#[test]
fn skip_header_keeps_binary_entries_out_of_following_text_file() {
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

    let (temp_dir, output) =
        run_diff_splitter(diff.as_bytes(), &["--mask-linenum", "--skip-header"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        read_output_bytes(&temp_dir, "__BINARY_FILES__.txt"),
        b"Binary files a/jdk/make/closed/tools/crypto/jce/sunjce_provider.jar and b/jdk/make/closed/tools/crypto/jce/sunjce_provider.jar differ\n"
    );
    assert_eq!(
        list_output_files(&temp_dir),
        vec![
            "__BINARY_FILES__.txt",
            "jdk/make/closed/tools/source-bundles/exclude-all"
        ]
    );

    let output_contents = read_output(
        &temp_dir,
        "jdk/make/closed/tools/source-bundles/exclude-all",
    );

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
