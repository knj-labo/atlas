//! Print a TOON (default) or JSON report for Astro and ESM JS/TS sources.
use std::{
    error::Error,
    ffi::OsString,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

const USAGE: &str = "usage: atlas-report <dir> [--format toon|json]";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Toon,
    Json,
}

fn arguments(args: impl IntoIterator<Item = OsString>) -> Result<(PathBuf, Format), String> {
    let args: Vec<_> = args.into_iter().collect();
    let (root, format) = match args.as_slice() {
        [root] => (root, Format::Toon),
        [root, flag, format] if flag == "--format" => (
            root,
            match format.to_str() {
                Some("toon") => Format::Toon,
                Some("json") => Format::Json,
                _ => {
                    return Err(format!(
                        "Unknown report format: {}\n{USAGE}",
                        format.to_string_lossy()
                    ));
                }
            },
        ),
        _ => return Err(USAGE.into()),
    };
    if root.to_string_lossy().starts_with('-') {
        return Err(USAGE.into());
    }
    Ok((root.into(), format))
}

fn collect(dir: &Path, out: &mut Vec<(PathBuf, String)>) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            if !matches!(
                entry.file_name().to_str(),
                Some(".git" | "node_modules" | "target" | "dist" | ".astro")
            ) {
                collect(&path, out)?;
            }
        } else if kind.is_file()
            && matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("astro" | "js" | "mjs" | "jsx" | "ts" | "mts" | "tsx")
            )
        {
            let source =
                fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            out.push((path, source));
        }
    }
    Ok(())
}

fn run(
    args: impl IntoIterator<Item = OsString>,
    mut output: impl Write,
) -> Result<(), Box<dyn Error>> {
    let (root, format) = arguments(args)?;
    if !fs::symlink_metadata(&root)?.file_type().is_dir() {
        return Err(format!(
            "{}: expected a directory, without following symlinks",
            root.display()
        )
        .into());
    }
    let mut sources = Vec::new();
    collect(&root, &mut sources)?;
    let report = atlas::analyze_project(sources).report();
    let encoded = match format {
        Format::Toon => toon_format::encode_default(&report)?,
        Format::Json => serde_json::to_string_pretty(&report)?,
    };
    writeln!(output, "{encoded}")?;
    Ok(())
}

fn main() -> ExitCode {
    match run(std::env::args_os().skip(1), io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("atlas-report: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let root = std::env::temp_dir().join(format!(
                "atlas-report-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn argument_errors_are_explicit() {
        for args in [
            vec![],
            vec!["--format"],
            vec!["root", "--format"],
            vec!["root", "--unknown", "json"],
            vec!["root", "--format", "xml"],
            vec!["root", "extra"],
            vec!["root", "--format", "json", "extra"],
        ] {
            assert!(arguments(args.into_iter().map(OsString::from)).is_err());
        }
        assert_eq!(arguments([OsString::from("root")]).unwrap().1, Format::Toon);
    }

    #[test]
    fn collection_supports_esm_extensions_and_skips_generated_directories_and_links() {
        let fixture = Fixture::new();
        for name in [
            "Card.astro",
            "a.js",
            "a.mjs",
            "a.jsx",
            "a.ts",
            "a.mts",
            "a.tsx",
            "a.d.ts",
            "a.d.mts",
            "skip.cjs",
            "skip.cts",
            "skip.d.cts",
            "README.md",
        ] {
            fs::write(fixture.0.join(name), "").unwrap();
        }
        for name in [".git", "node_modules", "target", "dist", ".astro"] {
            fs::create_dir(fixture.0.join(name)).unwrap();
            fs::write(fixture.0.join(name).join("bad.ts"), "const =;").unwrap();
        }
        fs::create_dir(fixture.0.join("src")).unwrap();
        fs::write(fixture.0.join("src/nested.ts"), "export const value = 1;").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&fixture.0, fixture.0.join("cycle")).unwrap();
            std::os::unix::fs::symlink(fixture.0.join("a.ts"), fixture.0.join("link.ts")).unwrap();
        }
        let mut sources = Vec::new();
        collect(&fixture.0, &mut sources).unwrap();
        assert_eq!(sources.len(), 10);
        assert!(
            sources
                .iter()
                .any(|(path, _)| path.ends_with("src/nested.ts"))
        );
    }

    #[test]
    fn default_toon_explicit_toon_and_json_encode_the_same_report() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("a.ts"), "export const value = 1;").unwrap();
        fs::write(
            fixture.0.join("Page.astro"),
            "---\nimport { value } from './a.ts';\n---\n<p>{value}</p>",
        )
        .unwrap();
        let mut toon = Vec::new();
        run([fixture.0.as_os_str().to_owned()], &mut toon).unwrap();
        let mut explicit_toon = Vec::new();
        run(
            [
                fixture.0.as_os_str().to_owned(),
                "--format".into(),
                "toon".into(),
            ],
            &mut explicit_toon,
        )
        .unwrap();
        assert_eq!(toon, explicit_toon);
        let mut json = Vec::new();
        run(
            [
                fixture.0.as_os_str().to_owned(),
                "--format".into(),
                "json".into(),
            ],
            &mut json,
        )
        .unwrap();
        let toon: serde_json::Value =
            toon_format::decode_strict(std::str::from_utf8(&toon).unwrap()).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert_eq!(toon, json);
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["files"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn io_failures_return_errors_without_partial_reports() {
        let fixture = Fixture::new();
        let mut out = Vec::new();
        assert!(run([fixture.0.join("missing").into_os_string()], &mut out).is_err());
        fs::write(fixture.0.join("invalid.ts"), [0xff]).unwrap();
        assert!(run([fixture.0.clone().into_os_string()], &mut out).is_err());
        assert!(run([fixture.0.join("invalid.ts").into_os_string()], &mut out).is_err());
        assert!(out.is_empty());
        #[cfg(unix)]
        {
            let link = fixture.0.join("link");
            std::os::unix::fs::symlink(&fixture.0, &link).unwrap();
            assert!(run([link.into_os_string()], &mut out).is_err());
        }
        struct FailedOutput;
        impl Write for FailedOutput {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("output failed"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let empty = Fixture::new();
        assert!(run([empty.0.clone().into_os_string()], FailedOutput).is_err());
    }
}
