//! A de-identification profile: the Basic Application Level Confidentiality Profile, the
//! Options a project chose, and what it changes on top. Read from JSON, so each deployment
//! writes its own; [`Profile::check`] refuses what pacs-rs cannot do.

use serde::Deserialize;

use crate::action::Action;

/// The Options of PS3.15 E.3 found in Table E.1-1, in its column order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Opt {
    RetainSafePrivate,
    RetainUids,
    RetainDeviceIdentity,
    RetainInstitutionIdentity,
    RetainPatientCharacteristics,
    RetainLongitudinalFullDates,
    RetainLongitudinalModifiedDates,
    CleanDescriptors,
    CleanStructuredContent,
    CleanGraphics,
}

impl Opt {
    /// The option's column in Table E.1-1, after the Basic Profile's.
    pub fn column(self) -> usize {
        self as usize
    }

    /// Its code in CID 7050 De-identification Method (PS3.16), scheme DCM.
    pub fn code(self) -> (&'static str, &'static str) {
        match self {
            Opt::RetainSafePrivate => ("113111", "Retain Safe Private Option"),
            Opt::RetainUids => ("113110", "Retain UIDs Option"),
            Opt::RetainDeviceIdentity => ("113109", "Retain Device Identity Option"),
            Opt::RetainInstitutionIdentity => ("113112", "Retain Institution Identity Option"),
            Opt::RetainPatientCharacteristics => {
                ("113108", "Retain Patient Characteristics Option")
            }
            Opt::RetainLongitudinalFullDates => (
                "113106",
                "Retain Longitudinal Temporal Information Full Dates Option",
            ),
            Opt::RetainLongitudinalModifiedDates => (
                "113107",
                "Retain Longitudinal Temporal Information Modified Dates Option",
            ),
            Opt::CleanDescriptors => ("113105", "Clean Descriptors Option"),
            Opt::CleanStructuredContent => ("113104", "Clean Structured Content Option"),
            Opt::CleanGraphics => ("113103", "Clean Graphics Option"),
        }
    }
}

/// Tags an override applies to: `(gggg,eeee)` with hexadecimal digits, any of which may be
/// `x` for "any digit", e.g. `(0019,xxxx)` for a private group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TagPattern {
    value: u32,
    mask: u32,
}

impl TagPattern {
    pub fn matches(&self, group: u16, element: u16) -> bool {
        let tag = (u32::from(group) << 16) | u32::from(element);
        tag & self.mask == self.value
    }
}

impl std::str::FromStr for TagPattern {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let digits: String = s
            .chars()
            .filter(|c| !matches!(c, '(' | ')' | ',' | ' '))
            .collect();
        if digits.len() != 8 {
            return Err(format!("{s:?} is not a tag written (gggg,eeee)"));
        }
        let (mut value, mut mask) = (0u32, 0u32);
        for c in digits.chars() {
            value <<= 4;
            mask <<= 4;
            if c == 'x' || c == 'X' {
                continue;
            }
            let digit = c
                .to_digit(16)
                .ok_or_else(|| format!("{s:?} is not a tag written (gggg,eeee)"))?;
            value |= digit;
            mask |= 0xF;
        }
        Ok(TagPattern { value, mask })
    }
}

impl std::fmt::Display for TagPattern {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let digit = |i: u32| {
            let shift = 28 - 4 * i;
            if (self.mask >> shift) & 0xF == 0 {
                'x'
            } else {
                char::from_digit((self.value >> shift) & 0xF, 16)
                    .expect("a nibble")
                    .to_ascii_uppercase()
            }
        };
        let g: String = (0..4).map(digit).collect();
        let e: String = (4..8).map(digit).collect();
        write!(f, "({g},{e})")
    }
}

impl<'de> Deserialize<'de> for TagPattern {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// One change to the table: `action` for every attribute `tag` matches, private ones
/// included, instead of what the Profile and Options say.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Override {
    pub tag: TagPattern,
    pub action: Action,
    /// Why, shown in De-identification Method beside the tag and action.
    #[serde(default)]
    pub reason: Option<String>,
}

/// Instances kept out of the archive, in quarantine, rather than de-identified: those that
/// may carry identification pacs-rs cannot remove (PS3.15 E.1.1: burned-in pixel data,
/// structured content, graphics, encapsulated documents).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Quarantine {
    /// SOP Class UIDs, or prefixes ending in "." for a family.
    #[serde(default = "Quarantine::default_sop_classes")]
    pub sop_classes: Vec<String>,
    /// Burned In Annotation (0028,0301) is YES.
    #[serde(default = "yes")]
    pub burned_in_annotation: bool,
    /// Recognizable Visual Features (0028,0302) is YES.
    #[serde(default)]
    pub recognizable_visual_features: bool,
    /// Image Type (0008,0008) has one of these values.
    #[serde(default = "Quarantine::default_image_types")]
    pub image_types: Vec<String>,
}

fn yes() -> bool {
    true
}

impl Quarantine {
    /// Secondary Capture (text and screens), encapsulated documents, Structured Reports and
    /// Presentation States (text the Clean Structured Content and Clean Graphics Options would
    /// have to clean), ultrasound (text often burned in).
    pub fn default_sop_classes() -> Vec<String> {
        [
            "1.2.840.10008.5.1.4.1.1.7",
            "1.2.840.10008.5.1.4.1.1.7.",
            "1.2.840.10008.5.1.4.1.1.104.",
            "1.2.840.10008.5.1.4.1.1.88.",
            "1.2.840.10008.5.1.4.1.1.11.",
            "1.2.840.10008.5.1.4.1.1.6.",
            "1.2.840.10008.5.1.4.1.1.3.1",
        ]
        .map(String::from)
        .to_vec()
    }

    pub fn default_image_types() -> Vec<String> {
        vec!["SCREEN SAVE".into()]
    }
}

impl Default for Quarantine {
    fn default() -> Self {
        Quarantine {
            sop_classes: Quarantine::default_sop_classes(),
            burned_in_annotation: true,
            recognizable_visual_features: false,
            image_types: Quarantine::default_image_types(),
        }
    }
}

/// A project's de-identification.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// Shown in De-identification Method.
    pub name: String,
    #[serde(default)]
    pub options: Vec<Opt>,
    #[serde(default)]
    pub overrides: Vec<Override>,
    /// The range a patient's date shift is drawn from, in days, with
    /// `retain-longitudinal-modified-dates`.
    #[serde(default = "Profile::default_date_shift")]
    pub date_shift_days: (i32, i32),
    /// Put before each patient's pseudonym, the new Patient ID and Patient's Name.
    #[serde(default)]
    pub pseudonym_prefix: String,
    #[serde(default)]
    pub quarantine: Quarantine,
    /// Take X wherever Table E.1-1 allows it ([`Action::resolve`]).
    #[serde(default)]
    pub remove_when_allowed: bool,
    /// Words that identify in this setting (a hospital's name, a city), removed with the
    /// patient's own from descriptions by `clean-descriptors`.
    #[serde(default)]
    pub extra_identifiers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProfileError {
    #[error("{0:?} is not implemented by pacs-rs; leave it out, and quarantine what needs it")]
    Unsupported(Opt),
    #[error("full dates and modified dates are two exclusive options (PS3.15 E.3.6)")]
    BothDateOptions,
    #[error("the date shift range must not include 0 days, which would keep real dates")]
    ZeroShift,
    #[error("profile: {0}")]
    Json(String),
}

impl Profile {
    fn default_date_shift() -> (i32, i32) {
        (-1095, -1)
    }

    /// The Basic Profile and nothing else, named `name`.
    pub fn basic(name: &str) -> Self {
        Profile {
            name: name.to_owned(),
            options: Vec::new(),
            overrides: Vec::new(),
            date_shift_days: Profile::default_date_shift(),
            pseudonym_prefix: String::new(),
            quarantine: Quarantine::default(),
            remove_when_allowed: false,
            extra_identifiers: Vec::new(),
        }
    }

    /// A profile from JSON, checked.
    pub fn from_json(json: &str) -> Result<Self, ProfileError> {
        let profile: Profile =
            serde_json::from_str(json).map_err(|e| ProfileError::Json(e.to_string()))?;
        profile.check()?;
        Ok(profile)
    }

    /// Refuse Options pacs-rs does not implement, and contradictions.
    pub fn check(&self) -> Result<(), ProfileError> {
        for opt in &self.options {
            if matches!(
                opt,
                Opt::RetainSafePrivate | Opt::CleanStructuredContent | Opt::CleanGraphics
            ) {
                return Err(ProfileError::Unsupported(*opt));
            }
        }
        if self.has(Opt::RetainLongitudinalFullDates)
            && self.has(Opt::RetainLongitudinalModifiedDates)
        {
            return Err(ProfileError::BothDateOptions);
        }
        let (a, b) = self.date_shift_days;
        if a.min(b) <= 0 && a.max(b) >= 0 {
            return Err(ProfileError::ZeroShift);
        }
        Ok(())
    }

    pub fn has(&self, opt: Opt) -> bool {
        self.options.contains(&opt)
    }
}
