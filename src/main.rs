use std::{
    fs::{self, File},
    io::{self, BufRead, BufReader, Write},
    path::PathBuf,
};
use regex::Regex;
use clap::Parser;

/// Splits a unified diff from standard input into individual files in a target directory.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// The directory where the output files will be created
    target_path: PathBuf,

    /// The number of leading path components to remove from the file paths found in the diff (default is auto-detect)
    #[arg(long, default_value_t = -1)]
    strip: i32,

    /// Flag to hide line numbers in '@@' hunk headers
    #[arg(long)]
    mask_linenum: bool,

    /// Flag to skip the diff header
    #[arg(long)]
    skip_header: bool,
}

fn main() -> io::Result<()> {
    let args = Args::parse();

    // Create the target directory if it doesn't already exist.
    fs::create_dir_all(&args.target_path)?;

    // --- In-Memory Diff Processing ---

    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin.lock());

    let mut binary_file_lines: Vec<String> = Vec::new();
    let mut current_file_lines: Vec<String> = Vec::new();
    let mut full_path: Option<PathBuf> = None;
    let mut is_binary = false;

    // Regex for generalizing @@ lines
    let re = Regex::new(r"(@@ -[0-9]+)(,[0-9]+)?( \+[0-9]+)(,[0-9]+)?( @@)").unwrap();
    // Regex for @@@ lines for "--cc" and "--combine"
    let re_combine = Regex::new(r"(@@@ -[0-9]+)(,[0-9]+)?( \-[0-9]+)(,[0-9]+)?( \+[0-9]+)(,[0-9]+)?( @@@)").unwrap();

    let mut buffer = Vec::new();
    let mut header_state = HeaderState::Diff;

    while reader.read_until(b'\n', &mut buffer)? != 0 {
        let line = String::from_utf8_lossy(&buffer).into_owned();

        match header_state {
            HeaderState::Diff => {
                if line.starts_with("diff --") {
                    if !current_file_lines.is_empty() && full_path.is_some() && !is_binary {
                        process_file_diff(&current_file_lines, full_path.as_ref().unwrap(), &args, &re, &re_combine)?;
                    }
                    current_file_lines.clear();
                    full_path = None;
                    is_binary = false;
                    current_file_lines.push(line.clone());
                    header_state = HeaderState::FromOrIndex;
                }
            }
            HeaderState::FromOrIndex => {
                if line.starts_with("index ") {
                    current_file_lines.push(line.clone());
                    header_state = HeaderState::From;
                } else if line.starts_with("--- ") {
                    current_file_lines.push(line.clone());
                    header_state = HeaderState::To;
                } else {
                    eprintln!("Error: Invalid diff format. Expected 'index ' or '--- ' line !!!!");
                    std::process::exit(1);
                }
            }
            HeaderState::From => {
                if line.starts_with("--- ") {
                    current_file_lines.push(line.clone());
                    header_state = HeaderState::To;
                } else {
                    eprintln!("Error: Invalid diff format. Expected '--- ' line !!!!");
                    std::process::exit(1);
                }
            }
            HeaderState::To => {
                if line.starts_with("+++ ") {
                    let path_str = extract_path(&line, "+++ ");
                    if !path_str.is_empty() {
                        full_path = Some(PathBuf::from(path_str));
                    }
                    current_file_lines.push(line.clone());
                    header_state = HeaderState::Body;
                } else {
                    eprintln!("Error: Invalid diff format. Expected '+++ ' line !!!!");
                    std::process::exit(1);
                }
            }
            HeaderState::Body => {
                if line.starts_with("diff --") {
                    if !current_file_lines.is_empty() && full_path.is_some() && !is_binary {
                        process_file_diff(&current_file_lines, full_path.as_ref().unwrap(), &args, &re, &re_combine)?;
                    }
                    current_file_lines.clear();
                    full_path = None;
                    is_binary = false;
                    current_file_lines.push(line.clone());
                    header_state = HeaderState::FromOrIndex;
                } else {
                    if line.starts_with("Binary files ") {
                        is_binary = true;
                        binary_file_lines.push(line.trim_end().to_string());
                    }
                    current_file_lines.push(line.clone());
                }
            }
        }

        buffer.clear();
    }

    // Process the last file's diff
    if !current_file_lines.is_empty() && full_path.is_some() && !is_binary {
        process_file_diff(&current_file_lines, full_path.as_ref().unwrap(), &args, &re, &re_combine)?;
    }

    if !binary_file_lines.is_empty() {
        let binary_files_path = args.target_path.join("__BINARY_FILES__.txt");
        let mut binary_files_file = File::create(&binary_files_path)?;
        for line in &binary_file_lines {
            writeln!(binary_files_file, "{}", line)?;
        }
    }

    println!("Processing complete. Files created in '{}'.", args.target_path.display());

    Ok(())
}

enum HeaderState {
    Diff,
    FromOrIndex,
    From,
    To,
    Body,
}

fn extract_path<'a>(line: &'a str, prefix: &str) -> &'a str {
    let line = line.trim_start_matches(prefix).trim();
    line.split('\t').next().unwrap_or(line)
}

fn process_file_diff(
    lines: &[String],
    full_path_buf: &PathBuf,
    args: &Args,
    re: &Regex,
    re_combine: &Regex,
) -> io::Result<()> {
    let from_path_str = lines
        .iter()
        .find(|line| line.starts_with("--- "))
        .map(|line| extract_path(line, "--- "))
        .unwrap_or("");

    let to_path_str = lines
        .iter()
        .find(|line| line.starts_with("+++ "))
        .map(|line| extract_path(line, "+++ "))
        .unwrap_or("");


    let strip_value = if args.strip == -1 {
        calculate_strip_value(from_path_str, to_path_str)
    } else {
        args.strip as usize
    };

    // --- Path Stripping Logic ---
    let stripped_path = if strip_value > 0 {
        let components: Vec<_> = full_path_buf.components().collect();
        if components.len() > strip_value {
            components[strip_value..].iter().collect::<PathBuf>()
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
    let output_file = args.target_path.join(&stripped_path);
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
            if args.skip_header && (trimmed_line.starts_with("diff --") || trimmed_line.starts_with("index ") || trimmed_line.starts_with("--- ") || trimmed_line.starts_with("+++ ")) {
                continue;
            }
            header_processed = true;
        }

        if args.mask_linenum {
            if trimmed_line.starts_with("@@ ") {
                let line_to_process;
                let mut line_remain = "";
                let parts: Vec<&str> = trimmed_line.splitn(3, "@@").collect();
                if parts.len() > 2 {
                    line_to_process = format!("@@{}@@", parts[1]);
                    line_remain = parts[2];
                } else {
                    line_to_process = trimmed_line.to_string();
                }

                let modified_line = re.replace_all(&line_to_process, |caps: &regex::Captures| {
                    let g1 = caps.get(1).map_or("", |m| m.as_str());
                    let g2 = caps.get(2).map_or("", |m| m.as_str());
                    let g3 = caps.get(3).map_or("", |m| m.as_str());
                    let g4 = caps.get(4).map_or("", |m| m.as_str());
                    let g5 = caps.get(5).map_or("", |m| m.as_str());

                    let g1_x = re_digit.replace_all(g1, "X");
                    let g3_x = re_digit.replace_all(g3, "X");

                    format!("{}{}{}{}{}{}\n", g1_x, g2, g3_x, g4, g5, line_remain)
                });
                write!(output_file_handle, "{}", modified_line)?;
            } else if trimmed_line.starts_with("@@@ ") {
                let line_to_process;
                let mut line_remain = "";
                let parts: Vec<&str> = trimmed_line.splitn(3, "@@@").collect();
                if parts.len() > 2 {
                    line_to_process = format!("@@@{}@@@", parts[1]);
                    line_remain = parts[2];
                } else {
                    line_to_process = trimmed_line.to_string();
                }

                let modified_line = re_combine.replace_all(&line_to_process, |caps: &regex::Captures| {
                    let g1 = caps.get(1).map_or("", |m| m.as_str());
                    let g2 = caps.get(2).map_or("", |m| m.as_str());
                    let g3 = caps.get(3).map_or("", |m| m.as_str());
                    let g4 = caps.get(4).map_or("", |m| m.as_str());
                    let g5 = caps.get(5).map_or("", |m| m.as_str());
                    let g6 = caps.get(6).map_or("", |m| m.as_str());
                    let g7 = caps.get(7).map_or("", |m| m.as_str());

                    let g1_x = re_digit.replace_all(g1, "X");
                    let g3_x = re_digit.replace_all(g3, "X");
                    let g5_x = re_digit.replace_all(g5, "X");

                    format!("{}{}{}{}{}{}{}{}\n", g1_x, g2, g3_x, g4, g5_x, g6, g7, line_remain)
                });
                write!(output_file_handle, "{}", modified_line)?;
            } else {
                write!(output_file_handle, "{}", line)?;
            }
        } else {
            write!(output_file_handle, "{}", line)?;
        }
    }

    Ok(())
}

fn calculate_strip_value(from_path: &str, to_path: &str) -> usize {
    let from_path_buf = PathBuf::from(from_path);
    let from_components: Vec<_> = from_path_buf.components().collect();
    let to_path_buf = PathBuf::from(to_path);
    let to_components: Vec<_> = to_path_buf.components().collect();

    let common_suffix_len = from_components.iter().rev().zip(to_components.iter().rev()).take_while(|(a, b)| a == b).count();

    from_components.len() - common_suffix_len
}
