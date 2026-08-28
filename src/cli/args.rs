//! A flag parser sized for this binary and nothing more.
//!
//! `clap` would be four lines of derive and roughly a dozen extra crates in the
//! dependency tree. This crate ships a signed, attested binary whose whole
//! selling point is a small auditable graph — and whose CI asserts the absence
//! of specific transitive dependencies — so the parser is hand-written, the way
//! `radiochron-agent` and `radiochron-fleet` already do it.
//!
//! Unknown flags are rejected rather than ignored, mirroring
//! `reject_unknown_arguments` on the MCP path: a mistyped `--durations` must
//! fail loudly instead of silently running with a default.

/// Flags parsed for one subcommand.
#[derive(Debug)]
pub(super) struct Flags {
    values: Vec<(String, String)>,
    switches: Vec<String>,
}

impl Flags {
    /// Parse `tokens`, accepting only the named flags.
    ///
    /// `value_flags` take an argument, written either `--name value` or
    /// `--name=value`. `switch_flags` stand alone.
    pub(super) fn parse(
        tokens: &[String],
        value_flags: &[&str],
        switch_flags: &[&str],
    ) -> anyhow::Result<Self> {
        let mut flags = Self {
            values: Vec::new(),
            switches: Vec::new(),
        };
        let mut index = 0;
        while index < tokens.len() {
            let token = tokens[index].as_str();
            let Some(body) = token.strip_prefix("--") else {
                anyhow::bail!(
                    "unexpected argument: {token}{}",
                    known(value_flags, switch_flags)
                );
            };

            if let Some((name, value)) = body.split_once('=') {
                if !value_flags.contains(&name) {
                    anyhow::bail!(
                        "{}{}",
                        unknown_or_valueless(name, switch_flags),
                        known(value_flags, switch_flags)
                    );
                }
                flags.set(name, value)?;
                index += 1;
                continue;
            }

            if value_flags.contains(&body) {
                let value = tokens.get(index + 1).ok_or_else(|| {
                    anyhow::anyhow!("--{body} needs a value, for example --{body} <value>")
                })?;
                if value.starts_with("--") {
                    anyhow::bail!("--{body} needs a value, but was followed by {value}");
                }
                flags.set(body, value)?;
                index += 2;
                continue;
            }

            if switch_flags.contains(&body) {
                if flags.switches.iter().any(|name| name == body) {
                    anyhow::bail!("--{body} was given twice");
                }
                flags.switches.push(body.to_string());
                index += 1;
                continue;
            }

            anyhow::bail!("unknown flag: --{body}{}", known(value_flags, switch_flags));
        }
        Ok(flags)
    }

    fn set(&mut self, name: &str, value: &str) -> anyhow::Result<()> {
        if self.values.iter().any(|(known, _)| known == name) {
            anyhow::bail!("--{name} was given twice");
        }
        self.values.push((name.to_string(), value.to_string()));
        Ok(())
    }

    /// The raw text of a value flag.
    pub(super) fn text(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(known, _)| known == name)
            .map(|(_, value)| value.as_str())
    }

    /// A value flag parsed as a signed integer.
    ///
    /// Bounds are deliberately not checked here. The MCP tool handlers already
    /// enforce them, and duplicating the ranges is how a CLI drifts from the
    /// protocol it fronts.
    pub(super) fn integer(&self, name: &str) -> anyhow::Result<Option<i64>> {
        self.text(name)
            .map(|value| {
                value
                    .parse::<i64>()
                    .map_err(|_| anyhow::anyhow!("--{name} must be a whole number, got {value}"))
            })
            .transpose()
    }

    /// Whether a switch flag was given.
    pub(super) fn on(&self, name: &str) -> bool {
        self.switches.iter().any(|known| known == name)
    }
}

fn unknown_or_valueless(name: &str, switch_flags: &[&str]) -> String {
    if switch_flags.contains(&name) {
        format!("--{name} is a switch and takes no value")
    } else {
        format!("unknown flag: --{name}")
    }
}

fn known(value_flags: &[&str], switch_flags: &[&str]) -> String {
    let mut names: Vec<String> = value_flags
        .iter()
        .map(|name| format!("--{name} <value>"))
        .chain(switch_flags.iter().map(|name| format!("--{name}")))
        .collect();
    if names.is_empty() {
        return "; this command takes no flags".to_string();
    }
    names.sort();
    format!("; accepted here: {}", names.join(", "))
}

#[cfg(test)]
mod tests {
    use super::Flags;

    fn tokens(input: &[&str]) -> Vec<String> {
        input.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn reads_both_value_spellings() {
        let flags = Flags::parse(
            &tokens(&["--detail", "full", "--max=12", "--refresh"]),
            &["detail", "max"],
            &["refresh"],
        )
        .unwrap();
        assert_eq!(flags.text("detail"), Some("full"));
        assert_eq!(flags.integer("max").unwrap(), Some(12));
        assert!(flags.on("refresh"));
        assert!(!flags.on("json"));
    }

    #[test]
    fn rejects_unknown_and_malformed_flags() {
        let cases: [(&[&str], &str); 5] = [
            (&["--durations", "5"], "unknown flag"),
            (&["--detail"], "needs a value"),
            (&["--detail", "--refresh"], "needs a value"),
            (&["--refresh=yes"], "takes no value"),
            (&["scan"], "unexpected argument"),
        ];
        for (input, expected) in cases {
            let error = Flags::parse(&tokens(input), &["detail"], &["refresh"])
                .expect_err("must reject")
                .to_string();
            assert!(error.contains(expected), "{error} lacks {expected}");
        }
    }

    #[test]
    fn rejects_repeats() {
        assert!(Flags::parse(&tokens(&["--max=1", "--max=2"]), &["max"], &[]).is_err());
        assert!(Flags::parse(&tokens(&["--refresh", "--refresh"]), &[], &["refresh"]).is_err());
    }

    #[test]
    fn negative_numbers_parse() {
        let flags = Flags::parse(&tokens(&["--threshold=-8"]), &["threshold"], &[]).unwrap();
        assert_eq!(flags.integer("threshold").unwrap(), Some(-8));
    }
}
