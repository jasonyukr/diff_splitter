use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new() -> Self {
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

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub fn run_diff_splitter(input: &[u8], args: &[&str]) -> (TempDir, Output) {
    let temp_dir = TempDir::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_diff_splitter"));
    for arg in args {
        command.arg(arg);
    }

    let mut child = command
        .arg(temp_dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    child.stdin.as_mut().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();

    (temp_dir, output)
}

pub fn read_output(temp_dir: &TempDir, relative_path: &str) -> String {
    fs::read_to_string(temp_dir.path().join(relative_path)).unwrap()
}

pub fn list_output_files(temp_dir: &TempDir) -> Vec<String> {
    let mut files = Vec::new();
    collect_files(temp_dir.path(), temp_dir.path(), &mut files);
    files.sort();
    files
}

fn collect_files(root: &Path, current: &Path, files: &mut Vec<String>) {
    for entry in fs::read_dir(current).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, files);
        } else {
            files.push(path.strip_prefix(root).unwrap().display().to_string());
        }
    }
}
