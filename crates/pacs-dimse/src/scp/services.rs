//! The services pacs-rs provides as an SCP, each with the SOP classes it answers and the
//! operation it takes (PS3.4): one table the association's messages are dispatched by, so
//! that a service is added here and in its handler, not in the association loop. And what
//! each AE title of pacs-rs offers of them ([`Capabilities`]).

use std::str::FromStr;

use crate::ae::AeTitle;
use crate::command::field;
use crate::{EXPLICIT_VR_LE, IMPLICIT_VR_LE, VERIFICATION};

/// A DIMSE service of the SCP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Service {
    /// C-ECHO on the Verification SOP Class (PS3.4 Annex A).
    Verification,
    /// C-STORE on the Storage SOP Classes (Annex B).
    Storage,
    /// C-FIND on the Query/Retrieve FIND SOP Classes (Annex C).
    Find,
    /// C-MOVE on the MOVE SOP Classes.
    Move,
    /// C-GET on the GET SOP Classes.
    Get,
}

impl Service {
    pub const ALL: [Service; 5] = [
        Service::Verification,
        Service::Storage,
        Service::Find,
        Service::Move,
        Service::Get,
    ];

    /// Its name in the settings.
    pub fn name(self) -> &'static str {
        match self {
            Service::Verification => "verification",
            Service::Storage => "storage",
            Service::Find => "find",
            Service::Move => "move",
            Service::Get => "get",
        }
    }

    /// The SOP classes it answers.
    pub fn sop_classes(self) -> Vec<&'static str> {
        use crate::query::{
            PATIENT_ROOT_FIND, PATIENT_ROOT_GET, PATIENT_ROOT_MOVE, STUDY_ROOT_FIND, STUDY_ROOT_GET,
        };
        use crate::scu::retrieve::STUDY_ROOT_MOVE;
        match self {
            Service::Verification => vec![VERIFICATION],
            Service::Storage => crate::sop_classes::STORAGE
                .iter()
                .map(|(uid, _)| *uid)
                .collect(),
            Service::Find => vec![STUDY_ROOT_FIND, PATIENT_ROOT_FIND],
            Service::Move => vec![STUDY_ROOT_MOVE, PATIENT_ROOT_MOVE],
            Service::Get => vec![STUDY_ROOT_GET, PATIENT_ROOT_GET],
        }
    }

    /// The request it takes (PS3.7 Table E.1-1).
    pub fn operation(self) -> u16 {
        match self {
            Service::Verification => field::C_ECHO_RQ,
            Service::Storage => field::C_STORE_RQ,
            Service::Find => field::C_FIND_RQ,
            Service::Move => field::C_MOVE_RQ,
            Service::Get => field::C_GET_RQ,
        }
    }

    /// The service a SOP class belongs to: the registry.
    pub fn of(sop_class: &str) -> Option<Service> {
        Service::ALL
            .into_iter()
            .find(|service| service.sop_classes().contains(&sop_class))
    }
}

impl FromStr for Service {
    type Err = String;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Service::ALL
            .into_iter()
            .find(|service| service.name() == name)
            .ok_or_else(|| {
                let names: Vec<&str> = Service::ALL.iter().map(|s| s.name()).collect();
                format!("{name:?} is not a service; one of {}", names.join(", "))
            })
    }
}

/// What one AE title of pacs-rs offers: some services, and, when restricted, some transfer
/// syntaxes only. An AE title given none offers all of them in every transfer syntax.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities {
    pub ae_title: AeTitle,
    pub services: Vec<Service>,
    /// `None`: every transfer syntax pacs-rs reads.
    pub transfer_syntaxes: Option<Vec<String>>,
}

impl Capabilities {
    pub fn allows(&self, service: Service) -> bool {
        self.services.contains(&service)
    }

    pub fn allows_transfer_syntax(&self, uid: &str) -> bool {
        self.transfer_syntaxes
            .as_ref()
            .is_none_or(|allowed| allowed.iter().any(|ts| ts == uid))
    }
}

impl FromStr for Capabilities {
    type Err = String;

    /// `AE_TITLE=service+service[/syntax|syntax]`: the services by name, the transfer
    /// syntaxes by UID or `uncompressed` (Explicit and Implicit VR Little Endian).
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bad =
            || format!("capabilities are AE_TITLE=service+service[/syntax|syntax], not {text:?}");
        let (ae_title, rest) = text.split_once('=').ok_or_else(bad)?;
        let ae_title = AeTitle::new(ae_title.trim()).map_err(|e| format!("{text:?}: {e}"))?;
        let (services, syntaxes) = match rest.split_once('/') {
            Some((services, syntaxes)) => (services, Some(syntaxes)),
            None => (rest, None),
        };
        let mut chosen = Vec::new();
        for name in services.split('+').map(str::trim) {
            let service: Service = name.parse().map_err(|e| format!("{text:?}: {e}"))?;
            if chosen.contains(&service) {
                return Err(format!("{text:?}: {name} is given twice"));
            }
            chosen.push(service);
        }
        let transfer_syntaxes = match syntaxes {
            None => None,
            Some(syntaxes) => {
                let mut uids = Vec::new();
                for syntax in syntaxes.split('|').map(str::trim) {
                    if syntax == "uncompressed" {
                        uids.extend([EXPLICIT_VR_LE.to_owned(), IMPLICIT_VR_LE.to_owned()]);
                    } else if pacs_core::is_valid_uid(syntax) {
                        uids.push(syntax.to_owned());
                    } else {
                        return Err(format!(
                            "{text:?}: {syntax:?} is neither a transfer syntax UID nor uncompressed"
                        ));
                    }
                }
                // every AE accepts Implicit VR Little Endian (PS3.5 10.1)
                if !uids.iter().any(|uid| uid == IMPLICIT_VR_LE) {
                    return Err(format!(
                        "{text:?}: Implicit VR Little Endian ({IMPLICIT_VR_LE}) is the default every AE takes, and cannot be left out"
                    ));
                }
                Some(uids)
            }
        };
        Ok(Capabilities {
            ae_title,
            services: chosen,
            transfer_syntaxes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sop_class_is_one_service() {
        for service in Service::ALL {
            for uid in service.sop_classes() {
                assert_eq!(Service::of(uid), Some(service), "{uid}");
            }
        }
        assert_eq!(Service::of("1.2.3"), None);
    }

    #[test]
    fn capabilities_are_read_from_the_settings() {
        let c: Capabilities = "RESEARCH=storage+verification/uncompressed"
            .parse()
            .unwrap();
        assert_eq!(c.ae_title.as_str(), "RESEARCH");
        assert!(c.allows(Service::Storage) && !c.allows(Service::Find));
        assert!(c.allows_transfer_syntax(IMPLICIT_VR_LE));
        assert!(!c.allows_transfer_syntax("1.2.840.10008.1.2.4.80"));
        let c: Capabilities = "QR=find+move+get".parse().unwrap();
        assert!(c.allows_transfer_syntax("1.2.840.10008.1.2.4.80"));
        for (bad, said) in [
            ("QR", "AE_TITLE=service"),
            ("QR=search", "is not a service"),
            ("QR=find+find", "given twice"),
            ("QR=find/jpeg", "neither a transfer syntax"),
            ("QR=find/1.2.840.10008.1.2.1", "cannot be left out"),
            ("=find", "AE title"),
        ] {
            let error = bad.parse::<Capabilities>().unwrap_err();
            assert!(error.contains(said), "{bad}: {error}");
        }
    }
}
