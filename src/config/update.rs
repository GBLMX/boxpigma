//! Update channel selection, matching the launcher's `[update]` block.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

/// Which release feed `boxpigma update` follows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    /// Stable releases only.
    #[default]
    Stable,
    /// Stable plus beta releases.
    Beta,
    /// Stable, beta, and alpha releases.
    Alpha,
}

impl UpdateChannel {
    pub const ALL: &'static [Self] = &[Self::Stable, Self::Beta, Self::Alpha];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
            Self::Alpha => "alpha",
        }
    }

    pub const fn manifest_url(self) -> &'static str {
        match self {
            Self::Stable => "https://updates.bionickernel.com/boxpigma-updates.json",
            Self::Beta => "https://updates.bionickernel.com/boxpigma-updates-beta.json",
            Self::Alpha => "https://updates.bionickernel.com/boxpigma-updates-alpha.json",
        }
    }
}

impl fmt::Display for UpdateChannel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

impl FromStr for UpdateChannel {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "stable" | "" => Ok(Self::Stable),
            "beta" => Ok(Self::Beta),
            "alpha" => Ok(Self::Alpha),
            other => Err(format!(
                "unknown update channel `{other}` (stable, beta, alpha)"
            )),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UpdateConfig {
    pub channel: UpdateChannel,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_round_trip_and_use_distinct_manifests() {
        assert_eq!(UpdateChannel::default(), UpdateChannel::Stable);
        for channel in UpdateChannel::ALL {
            assert_eq!(channel.name().parse::<UpdateChannel>(), Ok(*channel));
        }
        assert!(
            UpdateChannel::Stable
                .manifest_url()
                .ends_with("boxpigma-updates.json")
        );
        assert!(UpdateChannel::Beta.manifest_url().ends_with("-beta.json"));
        assert!(UpdateChannel::Alpha.manifest_url().ends_with("-alpha.json"));
    }

    #[test]
    fn a_config_without_update_uses_stable() {
        let config: UpdateConfig = toml_edit::de::from_str("").expect("empty config");
        assert_eq!(config.channel, UpdateChannel::Stable);
    }
}
