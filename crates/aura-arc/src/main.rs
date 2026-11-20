//! `AuraArc` — the AuraUI archiver.
//!
//! A console program with four verbs, the way every archiver people already know works:
//!
//! ```text
//! AuraArc a <архив.zip> <файлы и папки…>   добавить в архив (создать, если его нет)
//! AuraArc x <архив.zip> [куда]             распаковать
//! AuraArc l <архив.zip>                    список содержимого
//! AuraArc t <архив.zip>                    проверить, не распаковывая
//! ```
//!
//! It has no settings file, no service and no panel: one exe that works alone, so it can be
//! shipped inside the AuraUI bundle and handed out on its own. The Explorer starts it like a
//! person would, which is why it prints what it did and returns an exit code:
//! 0 — всё в порядке, 1 — не понял, что просят, 2 — что-то не получилось.

mod zip;

use std::path::{Path, PathBuf};

use zip::Summary;

const HELP: &str = "\
AuraArc — архиватор AuraUI (.zip)

  AuraArc a <архив.zip> <файлы и папки…>   добавить в архив; создаёт его, если нужно
  AuraArc x <архив.zip> [куда]             распаковать (по умолчанию — рядом с архивом)
  AuraArc l <архив.zip>                    показать содержимое
  AuraArc t <архив.zip>                    проверить архив, ничего не распаковывая

  --store    добавить без сжатия (быстрее, но архив больше)
  --quiet    не перечислять файлы, только итог
  --         дальше — имена, даже если они начинаются с дефиса
  --version  версия
  --help     эта подсказка

Коды возврата: 0 — всё в порядке, 1 — не понял команду, 2 — что-то не получилось.";

fn main() {
    console_utf8();
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(run(&args));
}

/// The Windows console starts in the system code page; Russian has to be asked for.
fn console_utf8() {
    use windows::Win32::System::Console::{SetConsoleCP, SetConsoleOutputCP};
    unsafe {
        let _ = SetConsoleOutputCP(65001);
        let _ = SetConsoleCP(65001);
    }
}

/// Options and words apart: `--` ends the options, so a file may be called `--store`.
fn parse(args: &[String]) -> (Options, Vec<String>, Option<i32>) {
    let mut options = Options::default();
    let mut words: Vec<String> = Vec::new();
    let mut literal = false;
    for arg in args {
        if literal {
            words.push(arg.clone());
            continue;
        }
        match arg.as_str() {
            "--" => literal = true,
            "--store" | "-0" => options.store = true,
            "--quiet" | "-q" => options.quiet = true,
            "--version" | "-V" => {
                println!("AuraArc {}", env!("CARGO_PKG_VERSION"));
                return (options, words, Some(0));
            }
            "--help" | "-h" | "/?" | "help" => {
                println!("{HELP}");
                return (options, words, Some(0));
            }
            other => words.push(other.to_string()),
        }
    }
    (options, words, None)
}

fn run(args: &[String]) -> i32 {
    let (options, words, early) = parse(args);
    if let Some(code) = early {
        return code;
    }
    let Some(verb) = words.first().map(|word| word.to_ascii_lowercase()) else {
        println!("{HELP}");
        return 1;
    };
    // People type `a`, `add`, and sometimes `create`; the long forms are the same command.
    let verb = match verb.as_str() {
        "a" | "add" | "создать" | "добавить" => "a",
        "x" | "extract" | "e" | "распаковать" => "x",
        "l" | "list" | "содержимое" => "l",
        "t" | "test" | "проверить" => "t",
        other => {
            eprintln!("AuraArc: не знаю команду «{other}».");
            println!("{HELP}");
            return 1;
        }
    };
    let Some(archive_word) = words.get(1) else {
        eprintln!("AuraArc: не хватает имени архива.");
        println!("{HELP}");
        return 1;
    };
    let archive = PathBuf::from(archive_word);
    match verb {
        "a" => {
            let inputs: Vec<PathBuf> = words[2..].iter().map(PathBuf::from).collect();
            if inputs.is_empty() {
                eprintln!("AuraArc: нечего добавить — укажите файлы или папки.");
                return 1;
            }
            match zip::add(&archive, &inputs, options.store, |entry, ok| {
                options.report(entry, ok, '+');
            }) {
                Ok(summary) => {
                    println!(
                        "Готово: {} в {} ({}{}{}).",
                        files(summary),
                        archive.display(),
                        size(summary.packed),
                        if summary.bytes > summary.packed { format!(" из {}", size(summary.bytes)) } else { String::new() },
                        trouble(summary),
                    );
                    exit_code(summary)
                }
                Err(error) => {
                    eprintln!("AuraArc: {error}");
                    2
                }
            }
        }
        "x" => {
            let dest = words.get(2).map(PathBuf::from).unwrap_or_else(|| beside(&archive));
            match zip::extract(&archive, &dest, |entry, ok| options.report(entry, ok, '←')) {
                Ok(summary) => {
                    println!(
                        "Готово: {} в {} ({}){}.",
                        files(summary),
                        dest.display(),
                        size(summary.bytes),
                        trouble(summary)
                    );
                    exit_code(summary)
                }
                Err(error) => {
                    eprintln!("AuraArc: {error}");
                    2
                }
            }
        }
        "l" => match zip::list(&archive) {
            Ok(entries) => {
                println!("{} — {}", archive.display(), entry_count(entries.len()));
                let mut total = 0u64;
                for entry in &entries {
                    total += entry.size;
                    if options.quiet {
                        continue;
                    }
                    let method = if entry.method == zip::STORED { "без сжатия" } else { "сжато" };
                    println!("  {:>12}  {:<10}  {}", size(entry.size), method, entry.name);
                }
                println!("Итого: {}, {}.", entry_count(entries.len()), size(total));
                0
            }
            Err(error) => {
                eprintln!("AuraArc: {error}");
                2
            }
        },
        _ => match zip::test(&archive, |entry, ok| options.report(entry, ok, '✓')) {
            Ok(summary) => {
                println!(
                    "Проверено: {}{}.",
                    files(summary),
                    trouble(summary)
                );
                exit_code(summary)
            }
            Err(error) => {
                eprintln!("AuraArc: {error}");
                2
            }
        },
    }
}

#[derive(Default)]
struct Options {
    store: bool,
    quiet: bool,
}

impl Options {
    fn report(&self, entry: &zip::Entry, ok: bool, mark: char) {
        if self.quiet {
            return;
        }
        if ok {
            println!("{mark} {:>12}  {}", size(entry.size), entry.name);
        }
    }
}

fn exit_code(summary: Summary) -> i32 {
    if summary.failed == 0 { 0 } else { 2 }
}

fn trouble(summary: Summary) -> String {
    if summary.failed == 0 {
        String::new()
    } else {
        format!("; не удалось: {}", plural(summary.failed, "файл", "файла", "файлов"))
    }
}

fn files(summary: Summary) -> String {
    plural(summary.files, "файл", "файла", "файлов")
}

fn entry_count(count: usize) -> String {
    plural(count, "запись", "записи", "записей")
}

/// Russian counts: 1 файл, 2 файла, 5 файлов — and 11 файлов, 21 файл.
fn plural(count: usize, one: &str, few: &str, many: &str) -> String {
    let last_two = count % 100;
    let last = count % 10;
    let word = if (11..=14).contains(&last_two) {
        many
    } else if last == 1 {
        one
    } else if (2..=4).contains(&last) {
        few
    } else {
        many
    };
    format!("{count} {word}")
}

/// Bytes the way a person reads them: 12,3 КБ.
fn size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let value = bytes as f64;
    let text = if value < KIB {
        format!("{bytes} Б")
    } else if value < KIB * KIB {
        format!("{:.1} КБ", value / KIB)
    } else if value < KIB * KIB * KIB {
        format!("{:.1} МБ", value / (KIB * KIB))
    } else {
        format!("{:.2} ГБ", value / (KIB * KIB * KIB))
    };
    text.replace('.', ",")
}

/// Where a person expects an archive to open: a folder named after the archive, beside it.
fn beside(archive: &Path) -> PathBuf {
    let name = archive.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_else(|| "архив".to_string());
    match archive.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(name),
        _ => PathBuf::from(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_read_the_russian_way() {
        assert_eq!(plural(1, "файл", "файла", "файлов"), "1 файл");
        assert_eq!(plural(2, "файл", "файла", "файлов"), "2 файла");
        assert_eq!(plural(5, "файл", "файла", "файлов"), "5 файлов");
        assert_eq!(plural(11, "файл", "файла", "файлов"), "11 файлов");
        assert_eq!(plural(21, "файл", "файла", "файлов"), "21 файл");
        assert_eq!(plural(0, "файл", "файла", "файлов"), "0 файлов");
    }

    #[test]
    fn sizes_read_the_russian_way() {
        assert_eq!(size(512), "512 Б");
        assert_eq!(size(1536), "1,5 КБ");
        assert_eq!(size(12 * 1024 * 1024), "12,0 МБ");
    }

    #[test]
    fn an_archive_opens_beside_itself() {
        assert_eq!(beside(Path::new("C:\\работа\\отпуск.zip")), Path::new("C:\\работа\\отпуск"));
        assert_eq!(beside(Path::new("отпуск.zip")), Path::new("отпуск"));
    }

    #[test]
    fn the_verbs_understand_plain_words() {
        assert_eq!(run(&["--help".to_string()]), 0);
        assert_eq!(run(&["совсем-не-команда".to_string()]), 1);
        assert_eq!(run(&["a".to_string()]), 1);
        assert_eq!(run(&["--version".to_string()]), 0);
    }

    #[test]
    fn a_name_that_looks_like_a_flag_is_still_a_name() {
        let words = parse(&["a".to_string(), "out.zip".to_string(), "--".to_string(), "--store".to_string()]).1;
        assert_eq!(words, vec!["a", "out.zip", "--store"]);
        assert_eq!(parse(&["a".to_string(), "out.zip".to_string(), "--store".to_string()]).1.len(), 2);
    }
}
