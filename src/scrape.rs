use std::collections::HashMap;
use std::io::Write;
use std::process::Command;

use regex::Regex;

#[derive(Debug, Clone)]
pub struct Option_ {
    pub flags: Vec<String>,
    pub values: Vec<String>,
    pub desc: String,
}

#[derive(Debug, Clone)]
pub struct Subcmd {
    pub desc: String,
    pub options: Vec<Option_>,
    pub positionals: HashMap<String, String>,
}

impl Subcmd {
    fn new(desc: String) -> Self {
        Self {
            desc,
            options: Vec::new(),
            positionals: HashMap::new(),
        }
    }
}

/// Run a command and return its --help output as lines.
fn read_help(cmd: &str, args: &[&str]) -> Vec<String> {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("Failed to run `{cmd} {}`: {e}", args.join(" ")));
    // Many tools write help to stderr, some to stdout. Combine both, prefer whichever is non-empty.
    let text = if output.stdout.is_empty() {
        String::from_utf8_lossy(&output.stderr).into_owned()
    } else {
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    text.lines().map(|l| l.to_string()).collect()
}

/// Extract flag names from a help line.
/// Handles `-f`, `--flag`, `+f`, `++flag` prefixes.
/// Strips braces, angle brackets, square brackets, and separators before scanning.
pub fn get_flags(line: &str) -> Vec<String> {
    let line = line.trim();
    // Strip content inside {}, <>, [] and replace separators
    let re_brace = Regex::new(r"\{[^}]*\}").unwrap();
    let re_angle = Regex::new(r"<[^>]*>").unwrap();
    let re_bracket = Regex::new(r"\[[^\]]*\]").unwrap();
    let re_seps = Regex::new(r"[,=/]+").unwrap();

    let mut s = line.to_string();
    for skip in [" or ", " and ", " OR ", " AND "] {
        s = s.replace(skip, " ");
    }
    s = re_brace.replace_all(&s, " ").to_string();
    s = re_angle.replace_all(&s, " ").to_string();
    s = re_bracket.replace_all(&s, " ").to_string();
    s = re_seps.replace_all(&s, " ").to_string();

    let words: Vec<&str> = s.split_whitespace().collect();
    let mut flags = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        if w.starts_with('-') || w.starts_with('+') {
            flags.push(w.to_string());
            // Allow 1 fully uppercase or lowercase word directly after a flag
            if i + 1 < words.len() {
                let next = words[i + 1];
                if !next.starts_with('-')
                    && !next.starts_with('+')
                    && (next == next.to_uppercase() || next == next.to_lowercase())
                {
                    i += 1;
                }
            }
        } else {
            break;
        }
        i += 1;
    }
    flags
}

/// Get the value following a flag, e.g. `{a,b}`, `<file>`, `VAL`, `[N]`.
/// Only matches structured value patterns, not description words.
pub fn get_flag_val(line: &str, flag: &str) -> String {
    let Some(pos) = line.find(flag) else {
        return String::new();
    };
    let rest = &line[pos + flag.len()..];

    // =VALUE (attached with equals)
    let re_eq = Regex::new(r"^=([\w[:punct:]]+)").unwrap();
    if let Some(caps) = re_eq.captures(rest) {
        return caps[1].trim_end_matches([',', '|']).to_string();
    }

    // <value> (angle brackets)
    let re_angle = Regex::new(r"^[ =]?(<[^>]+>)").unwrap();
    if let Some(caps) = re_angle.captures(rest) {
        return caps[1].to_string();
    }

    // {a,b} (braces with choices)
    let re_brace = Regex::new(r"^[ =]?(\{[^}]+\})").unwrap();
    if let Some(caps) = re_brace.captures(rest) {
        return caps[1].to_string();
    }

    // [val] (optional in brackets, immediately after flag)
    let re_opt = Regex::new(r"^[ =]?(\[[\w[:punct:]]+\])").unwrap();
    if let Some(caps) = re_opt.captures(rest) {
        return caps[1].to_string();
    }

    // ALLCAPS word after space (e.g. "--flag VALUE")
    let re_upper = Regex::new(r"^ ([A-Z][A-Z0-9_-]+)").unwrap();
    if let Some(caps) = re_upper.captures(rest) {
        return caps[1].to_string();
    }

    String::new()
}

/// Get human-readable description, assuming it's on the same line after flags/values.
pub fn get_option_desc(line: &str, tokens: &[&str]) -> String {
    let mut rest = line.to_string();
    for tok in tokens {
        if let Some(pos) = rest.find(tok) {
            rest = rest[pos + tok.len()..].to_string();
        }
    }
    let re_multi_space = Regex::new(r"  +").unwrap();
    re_multi_space
        .replace_all(rest.trim(), " ")
        .trim()
        .to_string()
}

/// Parse `{A,B}` or single values from flag value strings.
pub fn get_option_values(flag_values: &[String]) -> Vec<String> {
    let non_empty: Vec<&String> = flag_values.iter().filter(|v| !v.is_empty()).collect();
    if non_empty.is_empty() {
        return Vec::new();
    }
    // All non-empty values should be the same pattern, take the first unique one
    let val = non_empty[0];
    let re_word = Regex::new(r"\w+").unwrap();
    re_word
        .find_iter(val)
        .map(|m| m.as_str().to_string())
        .collect()
}

/// Parse options from help lines.
pub fn get_options(lines: &[String]) -> Vec<Option_> {
    let mut options = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        // Skip USAGE paragraph — only skip lines that look like usage syntax
        // continuation (don't start with a flag-like pattern after trimming)
        if lines[i].to_lowercase().starts_with("usage:") {
            i += 1; // skip the "usage:" line itself
            while i < lines.len() {
                let trimmed = lines[i].trim();
                if trimmed.is_empty() || trimmed.starts_with('-') || trimmed.starts_with('+') {
                    break;
                }
                i += 1;
            }
            continue;
        }
        if i >= lines.len() {
            break;
        }
        let line = &lines[i];
        let flags = get_flags(line);
        if flags.is_empty() {
            i += 1;
            continue;
        }
        let flag_values: Vec<String> = flags.iter().map(|f| get_flag_val(line, f)).collect();
        let tokens: Vec<&str> = flags
            .iter()
            .map(|s| s.as_str())
            .chain(flag_values.iter().map(|s| s.as_str()))
            .collect();
        let desc = get_option_desc(line, &tokens);
        let values = get_option_values(&flag_values);
        options.push(Option_ {
            flags,
            values,
            desc,
        });
        i += 1;
    }
    options
}

/// Detect subcommands: lines with `  name  description` pattern.
pub fn get_subcmds(lines: &[String]) -> HashMap<String, String> {
    let re = Regex::new(r"  ([a-zA-Z][a-zA-Z0-9_-]+)  +(.*)").unwrap();
    let mut subcmds = HashMap::new();
    for line in lines {
        if let Some(caps) = re.captures(line) {
            subcmds.insert(caps[1].to_string(), caps[2].to_string());
        }
    }
    subcmds
}

/// Detect positional arguments from lines like "Positionals:" followed by `{a,b,c}`.
pub fn get_positionals(lines: &[String]) -> Vec<String> {
    let mut positionals = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let lower = line.to_lowercase();
        if lower.trim() == "positionals:" || lower.trim() == "positional arguments:" {
            if i + 1 < lines.len() {
                let re = Regex::new(r"\{([a-z,]+)\}").unwrap();
                if let Some(caps) = re.captures(&lines[i + 1]) {
                    positionals.extend(caps[1].split(',').map(|s| s.to_string()));
                }
            }
        }
    }
    positionals
}

/// Full scrape: run `cmd --help`, parse options/subcommands/positionals, recurse into subcommands.
pub fn scrape_command(cmd: &str, help_flag: &str) -> (Vec<Option_>, HashMap<String, Subcmd>) {
    let help_lines = read_help(cmd, &[help_flag]);
    let options = get_options(&help_lines);
    let mut subcmd_descs = get_subcmds(&help_lines);
    let positionals = get_positionals(&help_lines);

    // Positionals get added as "subcommands" with empty description (matches Julia behaviour)
    for p in &positionals {
        subcmd_descs.entry(p.clone()).or_default();
    }

    let mut subcmds: HashMap<String, Subcmd> = subcmd_descs
        .into_iter()
        .map(|(k, v)| (k, Subcmd::new(v)))
        .collect();

    // Recursively parse each subcommand
    for (name, subcmd) in subcmds.iter_mut() {
        let sub_lines = read_help(cmd, &[name, help_flag]);
        subcmd.options = get_options(&sub_lines);
        subcmd.positionals = get_subcmds(&sub_lines);
        for p in get_positionals(&sub_lines) {
            subcmd.positionals.entry(p).or_default();
        }
    }

    (options, subcmds)
}

/// Escape a character in a string with backslash.
pub fn escape(s: &str, chars: &[char]) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if chars.contains(&c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Generate a complete .usage grammar from scraped data.
pub fn generate_usage<W: Write>(
    out: &mut W,
    cmd: &str,
    options: &[Option_],
    subcmds: &HashMap<String, Subcmd>,
) -> std::io::Result<()> {
    let mut placeholders: Vec<String> = Vec::new();

    let placeholder = |name: &str, placeholders: &mut Vec<String>| -> String {
        let clean = name.trim_start_matches('-');
        placeholders.push(clean.to_string());
        format!("<{clean}>")
    };

    // Format a single option for the grammar
    let format_option = |opt: &Option_, placeholders: &mut Vec<String>| -> String {
        let val_str = if opt.values.is_empty() {
            String::new()
        } else if opt.values.len() == 1 {
            format!(" {}", placeholder(&opt.values[0], placeholders))
        } else {
            let last_flag = opt.flags.last().unwrap().to_uppercase();
            format!(" {}", placeholder(&last_flag, placeholders))
        };

        let flags_str = opt
            .flags
            .iter()
            .map(|f| format!("{f}{val_str}"))
            .collect::<Vec<_>>()
            .join(" | ");
        let opt_str = format!("({flags_str})");

        if opt.desc.is_empty() {
            opt_str
        } else {
            format!("{opt_str} \"{}\"", escape(&opt.desc, &['"']))
        }
    };

    // Format a nonterminal rule: <NAME> ::= alt1 | alt2 | ...;
    let format_rule =
        |name: &str, alts: &[String], out: &mut W| -> std::io::Result<()> {
            if alts.is_empty() {
                return Ok(());
            }
            writeln!(out, "<{name}> ::=")?;
            for (i, alt) in alts.iter().enumerate() {
                if i == 0 {
                    writeln!(out, "\t  {alt}")?;
                } else {
                    writeln!(out, "\t| {alt}")?;
                }
            }
            writeln!(out, "\t;")?;
            Ok(())
        };

    // Main command definition
    if subcmds.is_empty() {
        write!(out, "{cmd} <PATH>...")?;
        if !options.is_empty() {
            write!(out, " || [<OPTION>]... <PATH>...")?;
        }
        writeln!(out, ";")?;
    } else {
        write!(out, "{cmd} <SUBCMD>")?;
        if !options.is_empty() {
            write!(out, " || [<OPTION>]... <SUBCMD>")?;
        }
        writeln!(out, ";")?;
    }
    writeln!(out)?;

    // Global options
    if !options.is_empty() {
        let alts: Vec<String> = options
            .iter()
            .map(|o| format_option(o, &mut placeholders))
            .collect();
        format_rule("OPTION", &alts, out)?;
        writeln!(out)?;
    }

    // Subcommands
    if !subcmds.is_empty() {
        // SUBCMD list
        let subcmd_refs: Vec<String> = subcmds
            .keys()
            .map(|k| placeholder(&k.to_uppercase(), &mut placeholders))
            .collect();
        writeln!(out, "<SUBCMD> ::= ({});", subcmd_refs.join(" | "))?;
        writeln!(out)?;

        // Per-subcmd rules
        let mut sorted_subcmds: Vec<_> = subcmds.iter().collect();
        sorted_subcmds.sort_by_key(|(k, _)| (*k).clone());

        for (name, subcmd) in &sorted_subcmds {
            let upper = name.to_uppercase();
            write!(out, "{} ::= {name}", placeholder(&upper, &mut placeholders))?;
            if !subcmd.desc.is_empty() {
                write!(out, " \"{}\"", escape(&subcmd.desc, &['"']))?;
            }
            if !subcmd.options.is_empty() {
                write!(out, " [<{upper}_OPTION>]...")?;
            }
            if !subcmd.positionals.is_empty() {
                write!(out, " [<{upper}_POSITIONAL>]...")?;
            }
            writeln!(out, ";")?;
        }
        writeln!(out)?;

        // Per-subcmd option and positional rules
        for (name, subcmd) in &sorted_subcmds {
            let upper = name.to_uppercase();
            if !subcmd.options.is_empty() {
                let alts: Vec<String> = subcmd
                    .options
                    .iter()
                    .map(|o| format_option(o, &mut placeholders))
                    .collect();
                format_rule(&format!("{upper}_OPTION"), &alts, out)?;
            }
            if !subcmd.positionals.is_empty() {
                let alts: Vec<String> = subcmd
                    .positionals
                    .iter()
                    .map(|(k, v)| {
                        if v.is_empty() {
                            k.clone()
                        } else {
                            format!("{k} \"{v}\"")
                        }
                    })
                    .collect();
                format_rule(&format!("{upper}_POSITIONAL"), &alts, out)?;
            }
        }
    }

    // Choice nonterminals for multi-value options
    let mut emitted_choices: Vec<String> = Vec::new();
    let emit_choices =
        |opt: &Option_, out: &mut W, emitted: &mut Vec<String>| -> std::io::Result<()> {
            if opt.values.len() > 1 {
                let ph_name = opt.flags.last().unwrap().to_uppercase();
                let ph_name = ph_name.trim_start_matches('-');
                let key = ph_name.to_string();
                if !emitted.contains(&key) {
                    let vals = opt.values.join(" | ");
                    writeln!(out, "<{ph_name}> ::= ({vals});")?;
                    emitted.push(key);
                }
            }
            Ok(())
        };

    let mut any_choices = false;
    for opt in options {
        if opt.values.len() > 1 {
            any_choices = true;
            break;
        }
    }
    if !any_choices {
        for subcmd in subcmds.values() {
            for opt in &subcmd.options {
                if opt.values.len() > 1 {
                    any_choices = true;
                    break;
                }
            }
            if any_choices {
                break;
            }
        }
    }

    if any_choices {
        writeln!(out)?;
    }

    for opt in options {
        emit_choices(opt, out, &mut emitted_choices)?;
    }
    for subcmd in subcmds.values() {
        for opt in &subcmd.options {
            emit_choices(opt, out, &mut emitted_choices)?;
        }
    }

    // FILE/PATH completions for placeholders that look like file references
    let file_phs: Vec<&String> = placeholders
        .iter()
        .filter(|p| p.to_lowercase().starts_with("file"))
        .collect();
    let unique_file_phs: Vec<&&String> = {
        let mut seen = Vec::new();
        let mut result = Vec::new();
        for ph in &file_phs {
            if !seen.contains(ph) {
                seen.push(ph);
                result.push(ph);
            }
        }
        result
    };
    if !unique_file_phs.is_empty() {
        writeln!(out)?;
        for ph in unique_file_phs {
            writeln!(out, "<{ph}> ::= <PATH>;")?;
        }
    }

    writeln!(out)?;
    writeln!(
        out,
        "# Fix PATH not adding space after completion"
    )?;
    writeln!(
        out,
        r#"<PATH@zsh> ::= {{{{ _path_files | sed '/\/$/!s/$/ /' }}}};"#
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_flags_simple() {
        assert_eq!(get_flags("-s"), vec!["-s"]);
    }

    #[test]
    fn test_get_flags_short_long_with_values() {
        assert_eq!(
            get_flags(" -s {a,b}, --long-flag={a,b}  Description text"),
            vec!["-s", "--long-flag"]
        );
    }

    #[test]
    fn test_get_flags_or_separator() {
        assert_eq!(get_flags("  -? or --help"), vec!["-?", "--help"]);
    }

    #[test]
    fn test_get_flags_angle_brackets() {
        assert_eq!(
            get_flags(" --solo_flag <file>  Description text that mention --another."),
            vec!["--solo_flag"]
        );
    }

    #[test]
    fn test_get_flags_plus_prefix() {
        assert_eq!(
            get_flags("++file FILE           include elements with matching file"),
            vec!["++file"]
        );
    }

    #[test]
    fn test_get_flags_slash_separator() {
        assert_eq!(get_flags("--git / --no-git"), vec!["--git", "--no-git"]);
    }

    #[test]
    fn test_get_flags_track_allocation() {
        assert_eq!(
            get_flags("--track-allocation=@<path>"),
            vec!["--track-allocation"]
        );
        assert_eq!(
            get_flags("--track-allocation[={none*|user|all}]"),
            vec!["--track-allocation"]
        );
    }

    #[test]
    fn test_get_flags_optional_value() {
        assert_eq!(
            get_flags("-o[N]   Open N windows (default: one per file)"),
            vec!["-o"]
        );
    }

    #[test]
    fn test_get_flags_logfile() {
        assert_eq!(
            get_flags("-l LOGFILE, --logfile LOGFILE"),
            vec!["-l", "--logfile"]
        );
    }

    #[test]
    fn test_get_flag_val_empty() {
        assert_eq!(get_flag_val("  -s", "-s"), "");
    }

    #[test]
    fn test_get_flag_val_braces() {
        assert_eq!(
            get_flag_val(" -s {a,b}, --long-flag={a,b}  Description text", "-s"),
            "{a,b}"
        );
        assert_eq!(
            get_flag_val(
                " -s {a,b}, --long-flag {a,b}  Description text",
                "--long-flag"
            ),
            "{a,b}"
        );
    }

    #[test]
    fn test_get_flag_val_angle() {
        assert_eq!(
            get_flag_val(
                " --solo_flag <file>  Description text that mention --another.",
                "--solo_flag"
            ),
            "<file>"
        );
    }

    #[test]
    fn test_get_flag_val_upper() {
        assert_eq!(
            get_flag_val(
                "++file FILE           include elements with matching file",
                "++file"
            ),
            "FILE"
        );
    }

    #[test]
    fn test_get_flag_val_optional() {
        assert_eq!(
            get_flag_val("-o[N]   Open N windows (default: one per file)", "-o"),
            "[N]"
        );
    }

    #[test]
    fn test_get_option_values_empty() {
        assert_eq!(
            get_option_values(&["".to_string(), "".to_string()]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn test_get_option_values_single() {
        assert_eq!(
            get_option_values(&["".to_string(), "VAL".to_string()]),
            vec!["VAL"]
        );
    }

    #[test]
    fn test_get_option_values_multiple() {
        assert_eq!(
            get_option_values(&["{A,B}".to_string(), "{A,B}".to_string()]),
            vec!["A", "B"]
        );
    }

    #[test]
    fn test_escape() {
        assert_eq!(escape(r#"say "hello""#, &['"']), r#"say \"hello\""#);
    }
}
