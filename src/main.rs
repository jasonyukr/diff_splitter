use clap::Parser;
use regex::Regex;
use std::{
    fs::{self, File},
    io::{self, BufRead, BufReader, Write},
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};

/// Splits a unified diff from standard input into individual files in a target directory.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// The directory where the output files will be created
    target_path: PathBuf,

    /// The number of leading path components to remove from the file paths found in the diff (default is auto-detect)
    #[arg(long, default_value_t = -1)]
    strip: i32,

    /// Flag to mask line numbers in '@@' or '@@@' hunk headers
    #[arg(long)]
    mask_linenum: bool,

    /// Flag to skip the diff header
    #[arg(long)]
    skip_header: bool,
}

fn main() -> io::Result<()> {
    let args = Args::parse();

    fs::create_dir_all(&args.target_path)?;

    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin.lock());

    let re = Regex::new(r"(@@ -[0-9]+)(,[0-9]+)?( \+[0-9]+)(,[0-9]+)?( @@)").unwrap();
    let re_combine =
        Regex::new(r"(@@@ -[0-9]+)(,[0-9]+)?( \-[0-9]+)(,[0-9]+)?( \+[0-9]+)(,[0-9]+)?( @@@)")
            .unwrap();

    let binary_file_lines = process_input(&mut reader, &args, &re, &re_combine)?;

    if !binary_file_lines.is_empty() {
        let binary_files_path = args.target_path.join("__BINARY_FILES__.txt");
        let mut binary_files_file = File::create(&binary_files_path)?;
        for line in &binary_file_lines {
            writeln!(binary_files_file, "{}", line)?;
        }
    }

    println!(
        "Processing complete. Files created in '{}'.",
        args.target_path.display()
    );
    Ok(())
}

fn process_input<R: BufRead>(
    reader: &mut R,
    args: &Args,
    re: &Regex,
    re_combine: &Regex,
) -> io::Result<Vec<String>> {
    let mut binary_file_lines: Vec<String> = Vec::new();
    let mut current_section = DiffSection::default();
    let mut parser_state = ParserState::Idle;
    let mut buffer = Vec::new();

    while reader.read_until(b'\n', &mut buffer)? != 0 {
        let line = decode_line(&buffer)?;

        match parser_state {
            ParserState::Idle => {
                if is_binary_summary_line(&line) {
                    binary_file_lines.push(line.trim_end().to_string());
                } else if is_diff_prelude_line(&line) {
                    current_section.start(line);
                    parser_state = ParserState::Header;
                } else if line.starts_with("--- ") {
                    current_section.start(line);
                    parser_state = ParserState::Header;
                } else if !line.trim().is_empty() {
                    return Err(invalid_diff_error("Invalid diff format", &line));
                }
            }
            ParserState::Header => {
                if is_diff_prelude_line(&line) {
                    flush_section(&mut current_section, args, re, re_combine)?;
                    current_section.start(line);
                } else if is_binary_summary_line(&line) {
                    binary_file_lines.push(line.trim_end().to_string());
                    current_section.clear();
                    parser_state = ParserState::Idle;
                } else {
                    current_section.push(line.clone());
                    if is_body_start_line(&line) {
                        parser_state = ParserState::Body;
                    }
                }
            }
            ParserState::Body => {
                if is_diff_prelude_line(&line) {
                    flush_section(&mut current_section, args, re, re_combine)?;
                    current_section.start(line);
                    parser_state = ParserState::Header;
                } else {
                    current_section.push(line);
                }
            }
        }

        buffer.clear();
    }

    flush_section(&mut current_section, args, re, re_combine)?;

    Ok(binary_file_lines)
}

#[derive(Default)]
struct DiffSection {
    lines: Vec<String>,
    header_from_path: Option<String>,
    header_to_path: Option<String>,
}

impl DiffSection {
    fn start(&mut self, line: String) {
        self.clear();
        self.capture_diff_header_paths(&line);
        self.lines.push(line);
    }

    fn push(&mut self, line: String) {
        self.capture_diff_header_paths(&line);
        self.lines.push(line);
    }

    fn clear(&mut self) {
        self.lines.clear();
        self.header_from_path = None;
        self.header_to_path = None;
    }

    fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    fn capture_diff_header_paths(&mut self, line: &str) {
        if let Some((from_path, to_path)) = parse_diff_header_paths(line) {
            self.header_from_path = from_path;
            self.header_to_path = to_path;
        }
    }
}

enum ParserState {
    Idle,
    Header,
    Body,
}

fn flush_section(
    section: &mut DiffSection,
    args: &Args,
    re: &Regex,
    re_combine: &Regex,
) -> io::Result<()> {
    if section.is_empty() || !section_has_processable_content(&section.lines) {
        section.clear();
        return Ok(());
    }

    let from_path_hint =
        find_header_path(&section.lines, "--- ").or_else(|| section.header_from_path.clone());
    let to_path_hint =
        find_header_path(&section.lines, "+++ ").or_else(|| section.header_to_path.clone());

    if let Some(output_path) =
        select_output_path(from_path_hint.as_deref(), to_path_hint.as_deref())
    {
        process_file_diff(
            &section.lines,
            &output_path,
            from_path_hint.as_deref(),
            to_path_hint.as_deref(),
            args,
            re,
            re_combine,
        )?;
    }

    section.clear();
    Ok(())
}

fn section_has_processable_content(lines: &[String]) -> bool {
    lines.iter().any(|line| {
        line.starts_with("--- ") || line.starts_with("+++ ") || is_body_start_line(line)
    })
}

fn decode_line(buffer: &[u8]) -> io::Result<String> {
    String::from_utf8(buffer.to_vec()).map_err(|err| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Input diff is not valid UTF-8: {err}"),
        )
    })
}

fn invalid_diff_error(message: &str, line: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{message}: {}", line.trim_end()),
    )
}

fn is_diff_prelude_line(line: &str) -> bool {
    line.starts_with("diff ")
}

fn is_binary_summary_line(line: &str) -> bool {
    let trimmed = line.trim_end();
    (trimmed.starts_with("Binary files ") || trimmed.starts_with("Files "))
        && trimmed.ends_with(" differ")
}

fn is_body_start_line(line: &str) -> bool {
    let trimmed = line.trim_end();
    trimmed.starts_with("@@ ") || trimmed.starts_with("@@@ ") || trimmed == "GIT binary patch"
}

fn parse_diff_header_paths(line: &str) -> Option<(Option<String>, Option<String>)> {
    if let Some(rest) = line.strip_prefix("diff --git ") {
        let tokens = parse_quoted_tokens(rest);
        if tokens.len() >= 2 {
            return Some((Some(tokens[0].clone()), Some(tokens[1].clone())));
        }
    }

    if let Some(rest) = line.strip_prefix("diff --cc ") {
        let tokens = parse_quoted_tokens(rest);
        if let Some(path) = tokens.first() {
            return Some((Some(path.clone()), Some(path.clone())));
        }
    }

    if let Some(rest) = line.strip_prefix("diff --combined ") {
        let tokens = parse_quoted_tokens(rest);
        if let Some(path) = tokens.first() {
            return Some((Some(path.clone()), Some(path.clone())));
        }
    }

    None
}

fn parse_quoted_tokens(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = input.trim().chars().peekable();

    while let Some(ch) = chars.peek() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }

        if *ch == '"' {
            chars.next();
            tokens.push(parse_quoted_token(&mut chars));
            continue;
        }

        let mut token = String::new();
        while let Some(ch) = chars.peek() {
            if ch.is_whitespace() {
                break;
            }
            token.push(*ch);
            chars.next();
        }
        tokens.push(token);
    }

    tokens
}

fn parse_quoted_token<I>(chars: &mut std::iter::Peekable<I>) -> String
where
    I: Iterator<Item = char>,
{
    let mut token = String::new();

    while let Some(ch) = chars.next() {
        if ch == '"' {
            break;
        }

        if ch == '\\' {
            token.push(parse_escaped_char(chars));
            continue;
        }

        token.push(ch);
    }

    token
}

fn parse_escaped_char<I>(chars: &mut std::iter::Peekable<I>) -> char
where
    I: Iterator<Item = char>,
{
    match chars.next() {
        Some('a') => '\u{0007}',
        Some('b') => '\u{0008}',
        Some('t') => '\t',
        Some('n') => '\n',
        Some('v') => '\u{000b}',
        Some('f') => '\u{000c}',
        Some('r') => '\r',
        Some('"') => '"',
        Some('\\') => '\\',
        Some(ch @ '0'..='7') => {
            let mut value = ch.to_digit(8).unwrap();
            for _ in 0..2 {
                match chars.peek() {
                    Some(next @ '0'..='7') => {
                        value = (value * 8) + next.to_digit(8).unwrap();
                        chars.next();
                    }
                    _ => break,
                }
            }

            char::from_u32(value).unwrap_or('\u{fffd}')
        }
        Some(other) => other,
        None => '\\',
    }
}

fn plain_diff_timestamp_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^(?P<path>.+?)\s+\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}(?:\.\d+)? [+-]\d{4}$")
            .unwrap()
    })
}

fn extract_path(line: &str, prefix: &str) -> String {
    let trimmed = line.trim_start_matches(prefix).trim();

    if trimmed.starts_with('"') {
        return parse_quoted_tokens(trimmed)
            .into_iter()
            .next()
            .unwrap_or_default();
    }

    if let Some((path, _)) = trimmed.split_once('\t') {
        return path.to_string();
    }

    if let Some(captures) = plain_diff_timestamp_re().captures(trimmed) {
        return captures["path"].to_string();
    }

    trimmed.to_string()
}

fn find_header_path(lines: &[String], prefix: &str) -> Option<String> {
    lines
        .iter()
        .filter(|line| line.starts_with(prefix))
        .map(|line| extract_path(line, prefix))
        .last()
}

fn select_output_path(from_path: Option<&str>, to_path: Option<&str>) -> Option<PathBuf> {
    [to_path, from_path]
        .into_iter()
        .flatten()
        .find(|path| !path.is_empty() && *path != "/dev/null")
        .map(PathBuf::from)
}

fn process_file_diff(
    lines: &[String],
    output_path_buf: &PathBuf,
    from_path_hint: Option<&str>,
    to_path_hint: Option<&str>,
    args: &Args,
    re: &Regex,
    re_combine: &Regex,
) -> io::Result<()> {
    let from_path = find_header_path(lines, "--- ")
        .or_else(|| from_path_hint.map(ToOwned::to_owned))
        .unwrap_or_default();
    let to_path = find_header_path(lines, "+++ ")
        .or_else(|| to_path_hint.map(ToOwned::to_owned))
        .unwrap_or_default();

    let strip_value = if args.strip == -1 {
        calculate_strip_value(&from_path, &to_path)
    } else {
        args.strip as usize
    };

    let stripped_path = strip_path(output_path_buf, strip_value);
    let sanitized_path = sanitize_output_path(&stripped_path)?;
    if sanitized_path.as_os_str().is_empty() {
        return Ok(());
    }

    let output_file = args.target_path.join(&sanitized_path);
    if let Some(parent) = output_file.parent() {
        fs::create_dir_all(parent)?;
    }

    let re_digit = Regex::new(r"[0-9]").unwrap();
    let mut output_file_handle = File::create(&output_file)?;
    let mut header_processed = false;

    for line in lines {
        let trimmed_line = line.trim_end();

        if !header_processed {
            if args.skip_header && is_skippable_header_line(trimmed_line) {
                continue;
            }
            header_processed = true;
        }

        if args.mask_linenum {
            if trimmed_line.starts_with("@@ ") {
                let mut line_remain = "";
                let line_to_process;
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

                    let g1_x = re_digit.replace_all(g1, "0");
                    let g3_x = re_digit.replace_all(g3, "0");

                    format!("{}{}{}{}{}{}\n", g1_x, g2, g3_x, g4, g5, line_remain)
                });
                write!(output_file_handle, "{}", modified_line)?;
            } else if trimmed_line.starts_with("@@@ ") {
                let mut line_remain = "";
                let line_to_process;
                let parts: Vec<&str> = trimmed_line.splitn(3, "@@@").collect();
                if parts.len() > 2 {
                    line_to_process = format!("@@@{}@@@", parts[1]);
                    line_remain = parts[2];
                } else {
                    line_to_process = trimmed_line.to_string();
                }

                let modified_line =
                    re_combine.replace_all(&line_to_process, |caps: &regex::Captures| {
                        let g1 = caps.get(1).map_or("", |m| m.as_str());
                        let g2 = caps.get(2).map_or("", |m| m.as_str());
                        let g3 = caps.get(3).map_or("", |m| m.as_str());
                        let g4 = caps.get(4).map_or("", |m| m.as_str());
                        let g5 = caps.get(5).map_or("", |m| m.as_str());
                        let g6 = caps.get(6).map_or("", |m| m.as_str());
                        let g7 = caps.get(7).map_or("", |m| m.as_str());

                        let g1_x = re_digit.replace_all(g1, "0");
                        let g3_x = re_digit.replace_all(g3, "0");
                        let g5_x = re_digit.replace_all(g5, "0");

                        format!(
                            "{}{}{}{}{}{}{}{}\n",
                            g1_x, g2, g3_x, g4, g5_x, g6, g7, line_remain
                        )
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

fn is_skippable_header_line(line: &str) -> bool {
    line.starts_with("diff ")
        || line.starts_with("index ")
        || line.starts_with("old mode ")
        || line.starts_with("new mode ")
        || line.starts_with("deleted file mode ")
        || line.starts_with("new file mode ")
        || line.starts_with("copy from ")
        || line.starts_with("copy to ")
        || line.starts_with("rename from ")
        || line.starts_with("rename to ")
        || line.starts_with("similarity index ")
        || line.starts_with("dissimilarity index ")
        || line.starts_with("mode ")
        || line.starts_with("--- ")
        || line.starts_with("+++ ")
}

fn strip_path(path: &Path, strip_value: usize) -> PathBuf {
    if strip_value == 0 {
        return path.to_path_buf();
    }

    let components: Vec<_> = path.components().collect();
    if components.len() > strip_value {
        components[strip_value..].iter().collect::<PathBuf>()
    } else {
        path.file_name()
            .map_or_else(PathBuf::new, |os_str| PathBuf::from(os_str))
    }
}

fn sanitize_output_path(path: &Path) -> io::Result<PathBuf> {
    let mut sanitized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::Normal(part) => sanitized.push(part),
            Component::CurDir | Component::RootDir => {}
            Component::ParentDir => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("Refusing to write outside target path: {}", path.display()),
                ));
            }
            Component::Prefix(_) => {}
        }
    }

    Ok(sanitized)
}

fn calculate_strip_value(from_path: &str, to_path: &str) -> usize {
    if let Some(strip_value) = strip_prefixed_git_pair(from_path, to_path) {
        return strip_value;
    }

    if from_path == "/dev/null" {
        return strip_prefixed_git_path(to_path);
    }

    if to_path == "/dev/null" {
        return strip_prefixed_git_path(from_path);
    }

    let from_parent_components = parent_components(from_path);
    let to_parent_components = parent_components(to_path);
    let common_parent_suffix_len = from_parent_components
        .iter()
        .rev()
        .zip(to_parent_components.iter().rev())
        .take_while(|(a, b)| a == b)
        .count();

    from_parent_components
        .len()
        .saturating_sub(common_parent_suffix_len)
}

fn parent_components(path: &str) -> Vec<String> {
    PathBuf::from(path)
        .parent()
        .map(|parent| {
            parent
                .components()
                .filter_map(|component| match component {
                    Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn strip_prefixed_git_pair(from_path: &str, to_path: &str) -> Option<usize> {
    let from_first = first_path_component(from_path)?;
    let to_first = first_path_component(to_path)?;
    if matches!(from_first.as_str(), "a" | "b") && matches!(to_first.as_str(), "a" | "b") {
        return Some(1);
    }

    None
}

fn strip_prefixed_git_path(path: &str) -> usize {
    first_path_component(path)
        .map(|component| usize::from(matches!(component.as_str(), "a" | "b")))
        .unwrap_or(0)
}

fn first_path_component(path: &str) -> Option<String> {
    PathBuf::from(path)
        .components()
        .find_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
}

#[cfg(test)]
mod tests {
    use super::{
        calculate_strip_value, decode_line, extract_path, is_skippable_header_line,
        sanitize_output_path,
    };
    use std::path::Path;

    #[test]
    fn auto_strip_handles_git_paths_for_modified_files() {
        assert_eq!(calculate_strip_value("a/src/main.rs", "b/src/main.rs"), 1);
    }

    #[test]
    fn auto_strip_handles_new_git_file_paths() {
        assert_eq!(
            calculate_strip_value("/dev/null", "b/jdk/make/closed/bundles.gmk"),
            1,
        );
    }

    #[test]
    fn auto_strip_handles_deleted_git_file_paths() {
        assert_eq!(calculate_strip_value("a/src/main.rs", "/dev/null"), 1);
    }

    #[test]
    fn auto_strip_preserves_parent_path_for_plain_diff_rename() {
        assert_eq!(
            calculate_strip_value("old/src/old.txt", "new/src/new.txt"),
            1,
        );
    }

    #[test]
    fn extract_path_decodes_quoted_git_headers() {
        assert_eq!(
            extract_path("+++ \"b/dir with space/file\\tname.txt\"", "+++ ",),
            "b/dir with space/file\tname.txt"
        );
    }

    #[test]
    fn extract_path_strips_plain_diff_timestamps() {
        assert_eq!(
            extract_path("+++ new/src/file.txt\t2024-01-01 00:00:01 +0000", "+++ ",),
            "new/src/file.txt"
        );
    }

    #[test]
    fn skip_header_recognizes_copy_and_dissimilarity_metadata() {
        assert!(is_skippable_header_line("copy from src/old.txt"));
        assert!(is_skippable_header_line("copy to src/new.txt"));
        assert!(is_skippable_header_line("dissimilarity index 72%"));
        assert!(is_skippable_header_line("diff -u old/file new/file"));
    }

    #[test]
    fn sanitize_output_path_rejects_parent_directory_components() {
        let err = sanitize_output_path(Path::new("../../escape.txt")).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn sanitize_output_path_strips_root_components() {
        assert_eq!(
            sanitize_output_path(Path::new("/tmp/work/file.txt")).unwrap(),
            Path::new("tmp/work/file.txt")
        );
    }

    #[test]
    fn decode_line_rejects_invalid_utf8() {
        let err = decode_line(&[0xff, b'\n']).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }
}
