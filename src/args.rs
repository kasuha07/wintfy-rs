use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Run,
    Install { startup: bool },
    Uninstall { purge: bool },
    TestNotification,
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub command: Command,
    pub config: Option<PathBuf>,
}

impl Args {
    pub fn parse() -> Result<Self, String> {
        Self::parse_from(std::env::args_os().skip(1))
    }

    pub fn parse_from<I, S>(iter: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: Into<std::ffi::OsString>,
    {
        let mut config = None;
        let mut command = Command::Run;
        let mut startup = false;
        let mut purge = false;
        let mut args = iter
            .into_iter()
            .map(Into::into)
            .collect::<Vec<std::ffi::OsString>>()
            .into_iter();

        while let Some(arg) = args.next() {
            let arg = arg.to_string_lossy();
            match arg.as_ref() {
                "--config" => {
                    let value = args
                        .next()
                        .ok_or_else(|| "--config requires a path".to_string())?;
                    config = Some(PathBuf::from(value));
                }
                "--install" => command = Command::Install { startup },
                "--startup" | "--start-with-windows" => {
                    startup = true;
                    if matches!(command, Command::Install { .. }) {
                        command = Command::Install { startup };
                    }
                }
                "--uninstall" => command = Command::Uninstall { purge },
                "--purge" => {
                    purge = true;
                    if matches!(command, Command::Uninstall { .. }) {
                        command = Command::Uninstall { purge };
                    }
                }
                "--test-notification" => command = Command::TestNotification,
                "--version" | "-V" => command = Command::Version,
                "--help" | "-h" => return Err(Self::usage()),
                other => return Err(format!("unknown argument: {other}\n\n{}", Self::usage())),
            }
        }

        command = match command {
            Command::Install { .. } => Command::Install { startup },
            Command::Uninstall { .. } => Command::Uninstall { purge },
            other => other,
        };

        Ok(Self { command, config })
    }

    pub fn usage() -> String {
        "Usage: wintfy-rs.exe [--config PATH] [--install [--startup] | --uninstall [--purge] | --test-notification | --version]".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_config_and_install() {
        let args = Args::parse_from(["--config", "C:\\tmp\\config.toml", "--install", "--startup"])
            .unwrap();
        assert_eq!(args.config, Some(PathBuf::from("C:\\tmp\\config.toml")));
        assert_eq!(args.command, Command::Install { startup: true });
    }
}
