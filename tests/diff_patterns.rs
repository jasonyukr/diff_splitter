mod common;

use common::{list_output_files, read_output, run_diff_splitter};

#[test]
fn plain_unified_diff_with_prelude_and_timestamps_is_split() {
    let diff = concat!(
        "diff -u old/src/file.txt new/src/file.txt\n",
        "--- old/src/file.txt\t2024-01-01 00:00:00 +0000\n",
        "+++ new/src/file.txt\t2024-01-01 00:00:01 +0000\n",
        "@@ -1 +1 @@\n",
        "-old\n",
        "+new\n",
    );

    let (temp_dir, output) = run_diff_splitter(diff.as_bytes(), &["--skip-header"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        read_output(&temp_dir, "src/file.txt"),
        concat!("@@ -1 +1 @@\n", "-old\n", "+new\n",)
    );
}

#[test]
fn deleted_git_diff_uses_the_preimage_path() {
    let diff = concat!(
        "diff --git a/src/deleted.txt b/src/deleted.txt\n",
        "deleted file mode 100644\n",
        "index 1234567..0000000\n",
        "--- a/src/deleted.txt\n",
        "+++ /dev/null\n",
        "@@ -1 +0,0 @@\n",
        "-gone\n",
    );

    let (temp_dir, output) = run_diff_splitter(diff.as_bytes(), &[]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(list_output_files(&temp_dir), vec!["src/deleted.txt"]);
}

#[test]
fn quoted_git_paths_are_unquoted_in_output_paths() {
    let diff = concat!(
        "diff --git \"a/dir with space/file name.txt\" \"b/dir with space/file name.txt\"\n",
        "index 1234567..89abcde 100644\n",
        "--- \"a/dir with space/file name.txt\"\n",
        "+++ \"b/dir with space/file name.txt\"\n",
        "@@ -1 +1 @@\n",
        "-old\n",
        "+new\n",
    );

    let (temp_dir, output) = run_diff_splitter(diff.as_bytes(), &[]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        list_output_files(&temp_dir),
        vec!["dir with space/file name.txt"]
    );
}

#[test]
fn git_rename_keeps_parent_directories_with_auto_strip() {
    let diff = concat!(
        "diff --git a/src/old.txt b/src/new.txt\n",
        "similarity index 100%\n",
        "rename from src/old.txt\n",
        "rename to src/new.txt\n",
        "--- a/src/old.txt\n",
        "+++ b/src/new.txt\n",
        "@@ -1 +1 @@\n",
        " same\n",
    );

    let (temp_dir, output) = run_diff_splitter(diff.as_bytes(), &["--skip-header"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(list_output_files(&temp_dir), vec!["src/new.txt"]);
    assert_eq!(
        read_output(&temp_dir, "src/new.txt"),
        "@@ -1 +1 @@\n same\n"
    );
}

#[test]
fn skip_header_drops_copy_and_dissimilarity_metadata() {
    let diff = concat!(
        "diff --git a/src/original.txt b/src/copied.txt\n",
        "dissimilarity index 72%\n",
        "copy from src/original.txt\n",
        "copy to src/copied.txt\n",
        "--- a/src/original.txt\n",
        "+++ b/src/copied.txt\n",
        "@@ -1 +1 @@\n",
        " same\n",
    );

    let (temp_dir, output) = run_diff_splitter(diff.as_bytes(), &["--skip-header"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        read_output(&temp_dir, "src/copied.txt"),
        "@@ -1 +1 @@\n same\n"
    );
}

#[test]
fn combined_all_paths_diff_is_split_and_masked() {
    let diff = concat!(
        "diff --cc src/file.txt\n",
        "index fabadb8,cc95eb0..4866510\n",
        "--- a/src/file.txt\n",
        "--- a/src/file.txt\n",
        "+++ b/src/file.txt\n",
        "@@@ -1,1 -1,1 +1,1 @@@\n",
        "- old1\n",
        " -old2\n",
        "++new\n",
    );

    let (temp_dir, output) =
        run_diff_splitter(diff.as_bytes(), &["--mask-linenum", "--skip-header"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        read_output(&temp_dir, "src/file.txt"),
        concat!(
            "@@@ -0,1 -0,1 +0,1 @@@\n",
            "- old1\n",
            " -old2\n",
            "++new\n",
        )
    );
}

#[test]
fn git_binary_patch_is_written_to_the_target_file() {
    let diff = concat!(
        "diff --git a/bin.dat b/bin.dat\n",
        "index 1111111..2222222 100644\n",
        "GIT binary patch\n",
        "literal 3\n",
        "abc\n",
        "literal 0\n",
    );

    let (temp_dir, output) = run_diff_splitter(diff.as_bytes(), &["--skip-header"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        read_output(&temp_dir, "bin.dat"),
        concat!("GIT binary patch\n", "literal 3\n", "abc\n", "literal 0\n",)
    );
}

#[test]
fn parent_directory_escape_paths_are_rejected() {
    let diff = concat!(
        "diff --git a/src/file.txt b/src/file.txt\n",
        "index 1234567..89abcde 100644\n",
        "--- a/src/file.txt\n",
        "+++ ../../escape.txt\n",
        "@@ -1 +1 @@\n",
        "-old\n",
        "+new\n",
    );

    let (_, output) = run_diff_splitter(diff.as_bytes(), &["--strip", "0"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Refusing to write outside target path")
    );
}
