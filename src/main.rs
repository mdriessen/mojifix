use clap::Parser;
use encoding_rs::WINDOWS_1252;
use std::{
    fs,
    io,
    path::{Path, PathBuf},
    process,
};

#[derive(Parser, Debug)]
#[command(
    name = "mojifix",
    version,
    about = "Safely repair common UTF-8/Windows-1252 mojibake"
)]
struct Args {
    /// Input UTF-8 file containing possible mojibake.
    input: PathBuf,

    /// Output file. Required unless --in-place or --dry-run is specified.
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Replace the input file atomically.
    #[arg(long)]
    in_place: bool,

    /// Show what would change without writing anything.
    #[arg(long)]
    dry_run: bool,

    /// Do not modify the file unless the repair looks highly confident.
    #[arg(long, default_value_t = true)]
    conservative: bool,
}

fn main() {
    let mut args = Args::parse();

    if let Err(err) = prompt_for_missing_action(&mut args) {
        eprintln!("error: {err}");
        process::exit(1);
    }

    if let Err(err) = run(&args) {
        eprintln!("error: {err}");
        process::exit(1);
    }
}

fn prompt_for_missing_action(args: &mut Args) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::IsTerminal;

    if args.dry_run || args.in_place || args.output.is_some() {
        return Ok(());
    }

    if !io::stdin().is_terminal() {
        return Err("specify --output FILE or use --in-place".into());
    }

    let choice = dialoguer::Select::new()
        .with_prompt("No action specified - what should mojifix do?")
        .items(&["Dry-run (show what would change)", "Repair in-place", "Write to a new file"])
        .default(0)
        .interact()?;

    match choice {
        0 => args.dry_run = true,
        1 => args.in_place = true,
        2 => {
            let path: String = dialoguer::Input::new()
                .with_prompt("Output file")
                .interact_text()?;
            args.output = Some(PathBuf::from(path.trim()));
        }
        _ => unreachable!(),
    }

    Ok(())
}

fn run(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    if args.in_place && args.output.is_some() {
        return Err("--in-place and --output cannot be used together".into());
    }

    if !args.dry_run && !args.in_place && args.output.is_none() {
        return Err("specify --output FILE or use --in-place".into());
    }

    let input_bytes = fs::read(&args.input)?;

    // Never silently reinterpret arbitrary binary data as text.
    let input = std::str::from_utf8(&input_bytes)
        .map_err(|e| format!("input is not valid UTF-8: {e}"))?;

    let (repaired, fixes) = repair_text(input, args.conservative);

    let changed = repaired.as_str() != input;

    if !changed {
        println!("No likely mojibake detected; file was not changed.");
        return Ok(());
    }

    let changes = fixes.len();

    println!(
        "Detected {} likely mojibake region(s).",
        changes
    );

    if args.dry_run {
        use rand::seq::SliceRandom;
        let mut sampled: Vec<&(String, String)> = fixes.iter().collect();
        let mut rng = rand::rng();
        sampled.shuffle(&mut rng);

        println!("Dry run: no files written. Sample fixes:");
        for (before, after) in sampled.iter().take(5) {
            println!("  {before} -> {after}");
        }
        return Ok(());
    }

    if args.in_place {
        atomic_replace(&args.input, repaired.as_bytes())?;
        println!("Repaired {}", args.input.display());
    } else {
        let output = args.output.as_ref().unwrap();

        if same_file(&args.input, output)? {
            return Err(
                "input and output refer to the same file; use --in-place".into()
            );
        }

        write_new_file(output, repaired.as_bytes())?;
        println!("Wrote {}", output.display());
    }

    Ok(())
}

/// Characters that are strong indicators of UTF-8 decoded as CP1252.
///
/// This intentionally does NOT include every possible character. The goal is
/// to avoid "fixing" legitimate multilingual text.
fn is_mojibake_marker(c: char) -> bool {
    matches!(
        c,
        'Ã' | 'Â' | 'Ð' | 'Ñ' | 'â' | 'ð' | '�'
    )
}

/// Repair a string conservatively.
///
/// The algorithm:
///
/// 1. Find suspicious characters.
/// 2. Take a small candidate region around them.
/// 3. Encode that region as Windows-1252.
/// 4. Decode the resulting bytes as UTF-8.
/// 5. Accept the repair only if:
///    - the resulting UTF-8 is valid, and
///    - it removes suspicious mojibake markers.
///
/// This prevents us from blindly transforming all text.
fn repair_text(input: &str, conservative: bool) -> (String, Vec<(String, String)>) {
    let mut result = String::with_capacity(input.len());
    let mut fixes = Vec::new();

    let mut chars = input.char_indices().peekable();
    let mut last_end = 0;

    while let Some((start, c)) = chars.next() {
        if !is_mojibake_marker(c) {
            continue;
        }

        // Find the end of a candidate region.
        //
        // Mojibake normally consists of a short run such as:
        //
        //     Ã©
        //     â€œ
        //     ðŸ˜Š
        //
        // We allow a little surrounding context so sequences can be repaired
        // together, but stop at whitespace/punctuation boundaries.
        let mut end = start + c.len_utf8();

        while let Some(&(idx, next)) = chars.peek() {
            if is_candidate_char(next) {
                chars.next();
                end = idx + next.len_utf8();
            } else if next.is_whitespace() {
                break;
            } else {
                break;
            }
        }

        // Include the original text before this region.
        result.push_str(&input[last_end..start]);

        let candidate = &input[start..end];

        match repair_candidate(candidate, conservative) {
            Some(fixed) => {
                if fixed != candidate {
                    fixes.push((candidate.to_string(), fixed.clone()));
                }
                result.push_str(&fixed);
            }
            None => {
                // No safe repair: preserve the original exactly.
                result.push_str(candidate);
            }
        }

        last_end = end;
    }

    result.push_str(&input[last_end..]);

    (result, fixes)
}

fn is_candidate_char(c: char) -> bool {
    // CP1252 mojibake is overwhelmingly composed of these characters.
    // Includes all Windows-1252 symbols that can appear as mojibake bytes
    // plus ASCII alphanumerics which are common trailing context.
    // Also includes C1 controls (0x80-0x9F) that appear in double-encoded
    // sequences (e.g. U+009D for 0x9D).
    if matches!(c, '\u{0080}'..='\u{009F}') {
        return true;
    }
    is_mojibake_marker(c)
        || matches!(
            c,
            '€'
                | '‚'
                | 'ƒ'
                | '„'
                | '…'
                | '†'
                | '‡'
                | 'ˆ'
                | '‰'
                | 'Š'
                | '‹'
                | 'Œ'
                | 'Ž'
                | '‘'
                | '’'
                | '“'
                | '”'
                | '•'
                | '–'
                | '—'
                | '˜'
                | '™'
                | 'š'
                | '›'
                | 'œ'
                | 'ž'
                | 'Ÿ'
                | ' ' // NBSP U+00A0, part of double-encoded Š (0x8A)
                | '¡'
                | '¢'
                | '£'
                | '¤'
                | '¥'
                | '¦'
                | '§'
                | '¨'
                | '©'
                | 'ª'
                | '«'
                | '¬'
                | '­'
                | '®'
                | '¯'
                | '°'
                | '±'
                | '²'
                | '³'
                | '´'
                | 'µ'
                | '¶'
                | '·'
                | '¸'
                | '¹'
                | 'º'
                | '»'
                | '¼'
                | '½'
                | '¾'
                | '¿'
                | 'À'
                | 'Á'
                | 'Â'
                | 'Ã'
                | 'Ä'
                | 'Å'
                | 'Æ'
                | 'Ç'
                | 'È'
                | 'É'
                | 'Ê'
                | 'Ë'
                | 'Ì'
                | 'Í'
                | 'Î'
                | 'Ï'
                | 'Ð'
                | 'Ñ'
                | 'Ò'
                | 'Ó'
                | 'Ô'
                | 'Õ'
                | 'Ö'
                | '×'
                | 'Ø'
                | 'Ù'
                | 'Ú'
                | 'Û'
                | 'Ü'
                | 'Ý'
                | 'Þ'
                | 'ß'
                | 'à'
                | 'á'
                | 'â'
                | 'ã'
                | 'ä'
                | 'å'
                | 'æ'
                | 'ç'
                | 'è'
                | 'é'
                | 'ê'
                | 'ë'
                | 'ì'
                | 'í'
                | 'î'
                | 'ï'
                | 'ð'
                | 'ñ'
                | 'ò'
                | 'ó'
                | 'ô'
                | 'õ'
                | 'ö'
                | '÷'
                | 'ø'
                | 'ù'
                | 'ú'
                | 'û'
                | 'ü'
                | 'ý'
                | 'þ'
                | 'ÿ'
                | '!'
                | '?'
                | '0'..='9'
                | 'A'..='Z'
                | 'a'..='z'
        )
}

/// Attempt to reverse CP1252 -> UTF-8 mojibake iteratively.
///
/// Example:
///
///     "Ã©"  -> C3 A9 -> "é"
///
/// Double-encoded mojibake such as "ÃƒÂ©" requires two iterations:
///     "ÃƒÂ©" -> "Ã©" -> "é"
/// Undefined Windows-1252 bytes are left as-is (returns None).
fn repair_candidate(candidate: &str, conservative: bool) -> Option<String> {
    let mut cur = candidate.to_string();
    let mut progressed = false;

    for _ in 0..5 {
        let (bytes, _, had_errors) = WINDOWS_1252.encode(&cur);

        if had_errors {
            // Leave undefined bytes as is.
            // If we already made progress, return what we have; otherwise no fix.
            // has true only for characters truly outside Windows-1252 (e.g. ☃).
            return if progressed { Some(cur) } else { None };
        }

        let fixed = match String::from_utf8(bytes.into_owned()) {
            Ok(s) => s,
            Err(_) => {
                // Invalid UTF-8 means we've decoded as far as possible.
                // Return the last valid cur if we progressed.
                return if progressed { Some(cur) } else { None };
            }
        };

        if fixed == cur {
            break;
        }

        progressed = true;
        cur = fixed;
    }

    if !progressed {
        return None;
    }

    if conservative {
        let before_score = mojibake_score(candidate);
        let after_score = mojibake_score(&cur);
        if before_score == 0 {
            return None;
        }
        if after_score >= before_score {
            return None;
        }
    }

    Some(cur)
}

/// A deliberately simple confidence score.
///
/// Higher means "looks more like mojibake".
fn mojibake_score(s: &str) -> usize {
    s.chars()
        .map(|c| match c {
            'Ã' | 'Â' | 'Ð' | 'Ñ' | 'â' | 'ð' => 2,
            '�' => 3,
            _ => 0,
        })
        .sum()
}

fn write_new_file(path: &Path, data: &[u8]) -> io::Result<()> {
    if path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("output file already exists: {}", path.display()),
        ));
    }

    fs::write(path, data)
}

/// Replace the input atomically where the platform permits it.
///
/// The original contents are written to a temporary file in the same
/// directory, then renamed over the original.
fn atomic_replace(path: &Path, data: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file");

    let tmp = parent.join(format!(".{file_name}.mojifix.tmp"));

    // Don't accidentally overwrite an existing temporary file.
    if tmp.exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("temporary file already exists: {}", tmp.display()),
        ));
    }

    fs::write(&tmp, data)?;

    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = fs::remove_file(&tmp);
            Err(err)
        }
    }
}

fn same_file(a: &Path, b: &Path) -> io::Result<bool> {
    let a = fs::canonicalize(a)?;
    let b = if b.exists() {
        fs::canonicalize(b)?
    } else {
        return Ok(false);
    };

    Ok(a == b)
}
