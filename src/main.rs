use std::{
    env,
    fs::{self, File},
    io::{self, BufRead, BufReader, Write},
    path::PathBuf,
};
use regex::Regex;

/// Splits a unified diff from standard input into individual files in a target directory.
///
/// Each file's diff hunk(s) are written to a corresponding file.
/// If `--hide-linenum` is provided, '@@' lines are generalized
/// (line numbers before commas replaced by 'X's).
///
/// Usage:
///   cat my_diff_file.diff | ./diff_splitter <target_directory> [strip_level] [--hide-linenum]
///
/// Arguments:
///   <target_directory>: The directory where the output files will be created.
///   [strip_level]: (Optional) The number of leading path components to remove
///                  from the file paths found in the diff. Defaults to 2.
///   [--hide-linenum]: (Optional) Flag to hide line numbers in '@@' hunk headers.
fn main() -> io::Result<()> {
    // --- 1. Argument and Environment Validation ---
    let args: Vec<String> = env::args().collect();
    let hide_linenum = args.iter().any(|arg| arg == "--hide-linenum");
    let filtered_args: Vec<String> = args.into_iter().filter(|arg| arg != "--hide-linenum").collect();

    if filtered_args.len() < 2 {
        eprintln!("Error: Target directory not specified.");
        eprintln!("Usage: {} <target_directory> [strip_level] [--hide-linenum]", filtered_args.get(0).map_or("diff_splitter", |s| s.as_str()));
        std::process::exit(1);
    }

    let target_dir = PathBuf::from(&filtered_args[1]);
    let strip_level: usize = filtered_args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2);

    // Create the target directory if it doesn't already exist.
    fs::create_dir_all(&target_dir)?;

    // --- 2. In-Memory Diff Processing ---

    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin.lock());

    let mut current_file_lines: Vec<String> = Vec::new();
    let mut full_path: Option<PathBuf> = None;
    let mut is_binary = false;

    // Regex for generalizing @@ lines
    let re = Regex::new(r"(@@ -[0-9]+)(,[0-9]+)?( \+[0-9]+)(,[0-9]+)?( @@)").unwrap();

    let mut buffer = Vec::new();
    let mut header_state = HeaderState::Diff;

    while reader.read_until(b'\n', &mut buffer)? != 0 {
        let line = String::from_utf8_lossy(&buffer).into_owned();

        match header_state {
            HeaderState::Diff => {
                if line.starts_with("diff --") {
                    if !current_file_lines.is_empty() && full_path.is_some() && !is_binary {
                        process_file_diff(&current_file_lines, full_path.as_ref().unwrap(), &target_dir, strip_level, &re, hide_linenum)?;
                    }
                    current_file_lines.clear();
                    full_path = None;
                    is_binary = false;
                    current_file_lines.push(line.clone());
                    header_state = HeaderState::From;
                }
            }
            HeaderState::From => {
                if line.starts_with("--- ") {
                    current_file_lines.push(line.clone());
                    header_state = HeaderState::To;
                } else {
                    eprintln!("Error: Invalid diff format. Expected '--- ' line.");
                    std::process::exit(1);
                }
            }
            HeaderState::To => {
                if line.starts_with("+++ ") {
                    let path_str = line.trim_end().trim_start_matches("+++ ").split('\t').next().unwrap_or("");
                    if !path_str.is_empty() {
                        full_path = Some(PathBuf::from(path_str));
                    }
                    current_file_lines.push(line.clone());
                    header_state = HeaderState::Body;
                } else {
                    eprintln!("Error: Invalid diff format. Expected '+++ ' line.");
                    std::process::exit(1);
                }
            }
            HeaderState::Body => {
                if line.starts_with("diff --") {
                    if !current_file_lines.is_empty() && full_path.is_some() && !is_binary {
                        process_file_diff(&current_file_lines, full_path.as_ref().unwrap(), &target_dir, strip_level, &re, hide_linenum)?;
                    }
                    current_file_lines.clear();
                    full_path = None;
                    is_binary = false;
                    current_file_lines.push(line.clone());
                    header_state = HeaderState::From;
                } else {
                    if line.starts_with("Binary files") {
                        is_binary = true;
                    }
                    current_file_lines.push(line.clone());
                }
            }
        }

        buffer.clear();
    }

    // Process the last file's diff
    if !current_file_lines.is_empty() && full_path.is_some() && !is_binary {
        process_file_diff(&current_file_lines, full_path.as_ref().unwrap(), &target_dir, strip_level, &re, hide_linenum)?;
    }

    println!("Processing complete. Files created in '{}'.", target_dir.display());

    Ok(())
}

enum HeaderState {
    Diff,
    From,
    To,
    Body,
}

fn process_file_diff(
    lines: &[String],
    full_path_buf: &PathBuf,
    target_dir: &PathBuf,
    strip_level: usize,
    re: &Regex,
    hide_linenum: bool,
) -> io::Result<()> {
    // --- Path Stripping Logic ---
    let stripped_path = if strip_level > 0 {
        let components: Vec<_> = full_path_buf.components().collect();
        if components.len() > strip_level {
            components[strip_level..].iter().collect::<PathBuf>()
        } else {
            full_path_buf.file_name().map_or_else(
                || PathBuf::from(""),
                |os_str| PathBuf::from(os_str),
            )
        }
    } else {
        full_path_buf.clone()
    };

    if stripped_path.as_os_str().is_empty() {
        return Ok(()); // Skip if the path is empty after stripping
    }
    
    // Ensure the parent directory for the output file exists
    let output_file = target_dir.join(&stripped_path);
    if let Some(parent) = output_file.parent() {
        fs::create_dir_all(parent)?;
    }

    let re_digit = Regex::new(r"[0-9]").unwrap();

    // --- File Creation and Content Writing ---
    let mut output_file_handle = File::create(&output_file)?;
    let mut header_processed = false;

    for line in lines {
        let trimmed_line = line.trim_end();

        if !header_processed {
            if trimmed_line.starts_with("diff --") || trimmed_line.starts_with("--- ") || trimmed_line.starts_with("+++ ") {
                continue;
            }
            header_processed = true;
        }

        // Process @@ lines
        if trimmed_line.starts_with("@@ ") && hide_linenum {
            let modified_line = re.replace_all(trimmed_line, |caps: &regex::Captures| {
                let g1 = caps.get(1).map_or("", |m| m.as_str());
                let g2 = caps.get(2).map_or("", |m| m.as_str());
                let g3 = caps.get(3).map_or("", |m| m.as_str());
                let g4 = caps.get(4).map_or("", |m| m.as_str());
                let g5 = caps.get(5).map_or("", |m| m.as_str());

                let g1_x = re_digit.replace_all(g1, "X");
                let g3_x = re_digit.replace_all(g3, "X");

                format!("{}{}{}{}{}\n", g1_x, g2, g3_x, g4, g5)
            });
            write!(output_file_handle, "{}", modified_line)?;
        } else {
            write!(output_file_handle, "{}", line)?;
        }
    }

    Ok(())
}

