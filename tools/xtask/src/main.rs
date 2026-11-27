mod packages;

#[path = "../../crates/chonk/src/cli.rs"]
mod chonk;
#[path = "../../crates/nap/src/cli.rs"]
mod nap;
#[path = "../../crates/scale/src/cli.rs"]
mod scale;
#[path = "../../crates/seal/src/cli.rs"]
mod seal;
#[path = "../../crates/whisker/src/cli.rs"]
mod whisker;

use anyhow::Result;
use clap::{Command, CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use std::{
    fs,
    path::{Path, PathBuf},
};

type CommandFactoryFn = fn() -> Command;

const TOOLS: [(&str, CommandFactoryFn); 5] = [
    ("chonk", chonk::Cli::command),
    ("scale", scale::Cli::command),
    ("nap", nap::Cli::command),
    ("whisker", whisker::Cli::command),
    ("seal", seal::Cli::command),
];

const SHELLS: [(Shell, &str, &str); 3] = [
    (Shell::Bash, "bash", "usr/share/bash-completion/completions"),
    (Shell::Zsh, "zsh", "usr/share/zsh/vendor-completions"),
    (Shell::Fish, "fish", "usr/share/fish/vendor_completions.d"),
];

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Task,
}

#[derive(Subcommand)]
enum Task {
    Completions {
        #[arg(long, default_value = "target/completions")]
        out: PathBuf,
    },
    StagePackages {
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        bin_dir: PathBuf,
    },
}

fn completions(out: &Path, package_layout: bool) -> Result<()> {
    for (shell, directory, package_directory) in SHELLS {
        let directory = out.join(if package_layout {
            package_directory
        } else {
            directory
        });
        fs::create_dir_all(&directory)?;
        for (name, command) in TOOLS {
            let filename = match shell {
                Shell::Bash => name.to_owned(),
                Shell::Zsh => format!("_{name}"),
                Shell::Fish => format!("{name}.fish"),
                _ => unreachable!(),
            };
            let mut script = Vec::new();
            generate(shell, &mut command(), name, &mut script);
            fs::write(
                directory.join(filename),
                normalize_text(std::str::from_utf8(&script)?),
            )?;
        }
    }
    Ok(())
}

fn normalize_text(text: &str) -> String {
    text.strip_prefix('\u{feff}')
        .unwrap_or(text)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Task::Completions { out } => completions(&out, false),
        Task::StagePackages { out, bin_dir } => packages::stage(&out, &bin_dir),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::read::GzDecoder;
    use std::io::Read;

    #[test]
    fn generates_completions_for_each_tool_and_shell() -> Result<()> {
        let output = tempfile::tempdir()?;
        completions(output.path(), false)?;
        for (name, _) in TOOLS {
            for file in [
                format!("bash/{name}"),
                format!("zsh/_{name}"),
                format!("fish/{name}.fish"),
            ] {
                assert!(fs::read_to_string(output.path().join(file))?.contains(name));
            }
        }
        Ok(())
    }

    #[test]
    fn stages_debian_metadata_and_documentation() -> Result<()> {
        let work = tempfile::tempdir()?;
        let bin = work.path().join("bin");
        let out = work.path().join("out");
        fs::create_dir(&bin)?;
        for (name, _) in TOOLS {
            fs::write(bin.join(name), b"abc")?;
        }
        packages::stage(&out, &bin)?;

        let metrics: serde_json::Value =
            serde_json::from_slice(&fs::read(out.join("metrics.json"))?)?;
        for (name, config, wrapper) in [
            ("chonkos-desktop", "chonkos.toml", "sway-session"),
            ("chonkos-desktop-x11", "chonkos-x11.toml", "i3-session"),
        ] {
            let staged = out.join(name);
            let greetd = staged.join("etc/greetd").join(config);
            assert!(fs::read_to_string(&greetd)?.contains(env!("CARGO_PKG_VERSION")));
            assert!(!greetd.with_extension("toml.in").exists());
            let wrapper = format!("usr/libexec/chonkos/{wrapper}");
            assert!(fs::read_to_string(staged.join("DEBIAN/md5sums"))?.contains(&wrapper));
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(
                    fs::metadata(staged.join(wrapper))?.permissions().mode() & 0o777,
                    0o755
                );
            }
            let size = metrics["packages"][name]["installed_size_kib"]
                .as_u64()
                .unwrap();
            assert!(size > 0);
            assert!(fs::read_to_string(staged.join("DEBIAN/control"))?
                .contains(&format!("Installed-Size: {size}\n")));
            let doc = staged.join("usr/share/doc").join(name);
            assert!(doc.join("copyright").is_file());
            assert!(doc.join("changelog.Debian.gz").is_file());
        }

        let package = out.join("chonkos-tools");
        let sums = fs::read_to_string(package.join("DEBIAN/md5sums"))?;
        for (name, _) in TOOLS {
            assert!(sums.contains(&format!(
                "900150983cd24fb0d6963f7d28e17f72  usr/bin/{name}\n"
            )));
        }
        let branding = out.join("chonkos-branding");
        let conffiles = fs::read_to_string(branding.join("DEBIAN/conffiles"))?;
        assert!(conffiles
            .lines()
            .any(|line| line == "/etc/dpkg/origins/chonkos"));
        let archive_keyring = out.join("chonkos-archive-keyring");
        assert!(!archive_keyring
            .join("etc/apt/sources.list.d/chonkos.sources")
            .exists());
        assert!(!archive_keyring.join("DEBIAN/conffiles").exists());

        for (path, expected) in [
            ("usr/share/man/man1/chonk.1.gz", ".TH"),
            (
                "usr/share/doc/chonkos-tools/changelog.Debian.gz",
                "chonkos-tools (",
            ),
        ] {
            let mut text = String::new();
            GzDecoder::new(fs::File::open(package.join(path))?).read_to_string(&mut text)?;
            assert!(text.contains(expected));
            assert!(!text.starts_with('\u{feff}'));
            assert!(!text.contains('\r'));
        }
        assert_eq!(normalize_text("\u{feff}a\r\nb\rc\n"), "a\nb\nc\n");
        Ok(())
    }
}
