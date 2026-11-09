//! Known World of Warcraft game flavors, keyed by the product code found in
//! each `_xxx_/.flavor.info` file (which is also the Battle.net `agentUID`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Flavor {
    /// Product code, e.g. `wow`, `wowt`, `wowf`, `wow_classic_era`.
    pub code: String,
}

struct Known {
    code: &'static str,
    name: &'static str,
    /// Inclusive range of `## Interface:` numbers this flavor loads.
    range: (u32, u32),
    /// TOC suffixes in priority order: game suffixes first, then family.
    suffixes: &'static [&'static str],
}

// Interface numbers are MMmmpp: Vanilla 1.15 => 11509, Forever 1.60 => 16001,
// TBC 2.5 => 20506, Wrath 3.4 => 30405, Cata 4.4 => 40402, Mists 5.5 => 50504,
// Mainline 12.1 => 120105. Forever and Classic Era share major 1, hence ranges.
const RETAIL: (u32, u32) = (100_000, 999_999);
const FOREVER: (u32, u32) = (16_000, 19_999);
const MISTS: (u32, u32) = (50_000, 59_999);
const WRATH: (u32, u32) = (30_000, 39_999);
const TBC: (u32, u32) = (20_000, 29_999);
const VANILLA: (u32, u32) = (10_000, 15_999);
const KNOWN: &[Known] = &[
    Known {
        code: "wow",
        name: "Retail",
        range: RETAIL,
        suffixes: &["Standard", "Mainline"],
    },
    Known {
        code: "wowt",
        name: "Retail PTR",
        range: RETAIL,
        suffixes: &["Standard", "Mainline"],
    },
    Known {
        code: "wowxptr",
        name: "Retail PTR 2",
        range: RETAIL,
        suffixes: &["Standard", "Mainline"],
    },
    Known {
        code: "wow_beta",
        name: "Retail Beta",
        range: RETAIL,
        suffixes: &["Standard", "Mainline"],
    },
    Known {
        code: "wowf",
        name: "Forever",
        range: FOREVER,
        suffixes: &["Forever", "Camelot", "Mainline"],
    },
    Known {
        code: "wowf_ptr",
        name: "Forever PTR",
        range: FOREVER,
        suffixes: &["Forever", "Camelot", "Mainline"],
    },
    Known {
        code: "wow_classic",
        name: "Classic (Mists)",
        range: MISTS,
        suffixes: &["Mists", "Classic"],
    },
    Known {
        code: "wow_classic_ptr",
        name: "Classic PTR",
        range: MISTS,
        suffixes: &["Mists", "Classic"],
    },
    Known {
        code: "wow_classic_beta",
        name: "Classic Beta",
        range: MISTS,
        suffixes: &["Mists", "Classic"],
    },
    Known {
        code: "wow_classic_era",
        name: "Classic Era",
        range: VANILLA,
        suffixes: &["Vanilla", "Classic"],
    },
    Known {
        code: "wow_classic_era_ptr",
        name: "Classic Era PTR",
        range: VANILLA,
        suffixes: &["Vanilla", "Classic"],
    },
    Known {
        code: "wow_anniversary",
        name: "Anniversary (TBC)",
        range: TBC,
        suffixes: &["TBC", "Classic"],
    },
    Known {
        code: "wow_classic_titan",
        name: "Titan Reforged (Wrath)",
        range: WRATH,
        suffixes: &["Wrath", "Classic"],
    },
];

impl Flavor {
    pub fn new(code: impl Into<String>) -> Self {
        Flavor { code: code.into() }
    }

    fn known(&self) -> Option<&'static Known> {
        KNOWN.iter().find(|k| k.code == self.code)
    }

    pub fn display_name(&self) -> String {
        match self.known() {
            Some(k) => k.name.to_string(),
            None => self.code.clone(),
        }
    }

    /// Does a TOC `## Interface:` number target this flavor?
    /// Unknown flavors accept everything so we never block installs.
    pub fn matches_interface(&self, interface: u32) -> bool {
        match self.known() {
            Some(k) => (k.range.0..=k.range.1).contains(&interface),
            None => true,
        }
    }

    pub fn toc_suffixes(&self) -> &'static [&'static str] {
        match self.known() {
            Some(k) => k.suffixes,
            None => &["Mainline"],
        }
    }

    /// Best-effort guess of a flavor from the `_xxx_` folder name when
    /// `.flavor.info` is missing.
    pub fn from_folder_name(folder: &str) -> Option<Flavor> {
        let code = match folder {
            "_retail_" => "wow",
            "_ptr_" => "wowt",
            "_xptr_" => "wowxptr",
            "_beta_" => "wow_beta",
            "_forever_" => "wowf",
            "_classic_" => "wow_classic",
            "_classic_ptr_" => "wow_classic_ptr",
            "_classic_beta_" => "wow_classic_beta",
            "_classic_era_" => "wow_classic_era",
            "_classic_era_ptr_" => "wow_classic_era_ptr",
            "_anniversary_" => "wow_anniversary",
            _ => return None,
        };
        Some(Flavor::new(code))
    }
}

/// Human-readable "12.1.5" form of an Interface number like 120105.
pub fn interface_to_version(i: u32) -> String {
    if i >= 10_000 {
        format!("{}.{}.{}", i / 10_000, (i / 100) % 100, i % 100)
    } else {
        i.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interface_matching() {
        assert!(Flavor::new("wowf").matches_interface(16001));
        assert!(!Flavor::new("wowf").matches_interface(120105));
        assert!(Flavor::new("wow").matches_interface(120105));
        assert!(Flavor::new("wow_classic_era").matches_interface(11509));
        assert!(!Flavor::new("wow_classic_era").matches_interface(16001));
        assert!(Flavor::new("wow_classic").matches_interface(50504));
        assert!(Flavor::new("mystery").matches_interface(1));
        assert_eq!(interface_to_version(120105), "12.1.5");
        assert_eq!(interface_to_version(16001), "1.60.1");
    }
}
