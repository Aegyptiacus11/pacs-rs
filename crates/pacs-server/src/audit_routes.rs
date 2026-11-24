//! Which requests are about studies, and how: what the audit trail must say of each route
//! of the REST API, DICOMweb and the web interface's own API.

use axum::http::Method;
use pacs_index::Level;

/// The data a request names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    /// A study, by its Study Instance UID (DICOMweb).
    Study(String),
    /// A resource of the REST API, by its level and public ID.
    Resource(Level, String),
    /// The resources of an archive made to order: public IDs of any level, read from the
    /// request (`/tools/create-archive`).
    Listed,
}

/// What a request does to the archive's data, as far as an audit trail is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Subject {
    /// A search: which studies, series or instances there are. (Query)
    Search,
    /// What a study is: its attributes, lists and counts, its measurements. (DICOM
    /// Instances Accessed, read)
    Viewed(Target),
    /// What is kept about a study changed: its measurements replaced, a label or a metadata
    /// value put on it or on a resource of it, or taken off. (DICOM Instances Accessed,
    /// update)
    Annotated(Target),
    /// Instances, frames or images of a study sent to the caller. (DICOM Instances
    /// Transferred, read)
    Sent(Target),
    /// Files to keep: a ZIP archive or a media file-set. (Export)
    Exported(Target),
    /// A resource and everything under it removed. (DICOM Study Deleted, or DICOM
    /// Instances Accessed, delete, when part of the study stays)
    Deleted(Level, String),
    /// Instances received. (DICOM Instances Transferred, create)
    Stores,
    /// A remote AE searched by C-FIND, by its id here. (Query, of that AE)
    RemoteSearch(String),
    /// A remote AE asked by C-MOVE to send studies here. (Begin Transferring DICOM
    /// Instances; what arrives is DICOM Instances Transferred, from the association)
    RemoteMove(String),
    /// Studies of this archive sent to a remote AE by C-STORE, by its id here. (Begin
    /// Transferring DICOM Instances, from this server)
    RemoteStore(String),
}

fn level(segment: &str) -> Option<Level> {
    Some(match segment {
        "patients" => Level::Patient,
        "studies" => Level::Study,
        "series" => Level::Series,
        "instances" => Level::Instance,
        _ => return None,
    })
}

/// What a DICOMweb request is, from its path under the DICOMweb root.
fn dicomweb(method: &Method, segments: &[&str]) -> Option<Subject> {
    let (get, post) = (*method == Method::GET, *method == Method::POST);
    match segments {
        // QIDO-RS from the root, and STOW-RS
        ["studies"] | ["series"] | ["instances"] if get => Some(Subject::Search),
        ["studies"] | ["studies", _] if post => Some(Subject::Stores),
        ["studies", study, rest @ ..] if get => {
            let target = Target::Study((*study).to_owned());
            // a study's lists of series and instances (QIDO-RS) and its metadata say what
            // it is; everything else retrieves it
            let describes = matches!(
                rest,
                ["series"] | ["instances"] | ["series", _, "instances"] | [.., "metadata"]
            );
            Some(if describes {
                Subject::Viewed(target)
            } else {
                Subject::Sent(target)
            })
        }
        _ => None,
    }
}

/// What a request is, or `None` when it is not about the archive's data. `mount` is the
/// DICOMweb root (`/dicom-web`).
pub(crate) fn subject(method: &Method, path: &str, mount: &str) -> Option<Subject> {
    let (get, post) = (*method == Method::GET, *method == Method::POST);
    if let Some(under) = path.strip_prefix(mount)
        && (under.is_empty() || under.starts_with('/'))
    {
        let segments: Vec<&str> = under.split('/').filter(|s| !s.is_empty()).collect();
        return dicomweb(method, &segments);
    }
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match segments.as_slice() {
        // the web interface's own lists and searches
        ["ui", "api", "patients"]
        | ["ui", "api", "studies"]
        | ["ui", "api", "studies", "facets"]
            if get =>
        {
            Some(Subject::Search)
        }
        ["ui", "api", "annotations", study] => {
            let target = Target::Study((*study).to_owned());
            match *method {
                Method::GET => Some(Subject::Viewed(target)),
                Method::PUT => Some(Subject::Annotated(target)),
                _ => None,
            }
        }
        ["modalities", remote, "studies"] if get => {
            Some(Subject::RemoteSearch((*remote).to_owned()))
        }
        ["modalities", remote, "move"] if post => Some(Subject::RemoteMove((*remote).to_owned())),
        ["modalities", remote, "store"] if post => Some(Subject::RemoteStore((*remote).to_owned())),
        ["tools", "find"] if post => Some(Subject::Search),
        ["tools", "create-archive"] if get || post => Some(Subject::Exported(Target::Listed)),
        ["instances"] if post => Some(Subject::Stores),
        // a restore indexes its files again, as an upload would
        ["trash", _, "restore"] if post => Some(Subject::Stores),
        [list] if get && level(list).is_some() => Some(Subject::Search),
        [kind, id, rest @ ..] => {
            let level = level(kind)?;
            let target = Target::Resource(level, (*id).to_owned());
            match (method.as_str(), rest) {
                ("DELETE", []) => Some(Subject::Deleted(level, (*id).to_owned())),
                ("PUT" | "DELETE", ["labels", _] | ["metadata", _]) => {
                    Some(Subject::Annotated(target))
                }
                ("GET", ["archive"] | ["media"]) => Some(Subject::Exported(target)),
                ("GET", ["file"]) => Some(Subject::Sent(target)),
                ("GET", _) => Some(Subject::Viewed(target)),
                _ => None,
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(method: Method, path: &str) -> Option<Subject> {
        subject(&method, path, "/dicom-web")
    }

    #[test]
    fn what_each_request_is() {
        use Subject::*;
        let study = || Target::Study("1.2.3".into());
        let rest = |level, id: &str| Target::Resource(level, id.into());
        let cases = [
            // DICOMweb
            (Method::GET, "/dicom-web/studies", Some(Search)),
            (Method::GET, "/dicom-web/series", Some(Search)),
            (Method::GET, "/dicom-web/instances", Some(Search)),
            (Method::POST, "/dicom-web/studies", Some(Stores)),
            (Method::POST, "/dicom-web/studies/1.2.3", Some(Stores)),
            (
                Method::GET,
                "/dicom-web/studies/1.2.3/series",
                Some(Viewed(study())),
            ),
            (
                Method::GET,
                "/dicom-web/studies/1.2.3/instances",
                Some(Viewed(study())),
            ),
            (
                Method::GET,
                "/dicom-web/studies/1.2.3/series/4.5/instances",
                Some(Viewed(study())),
            ),
            (
                Method::GET,
                "/dicom-web/studies/1.2.3/metadata",
                Some(Viewed(study())),
            ),
            (
                Method::GET,
                "/dicom-web/studies/1.2.3/series/4.5/metadata",
                Some(Viewed(study())),
            ),
            (
                Method::GET,
                "/dicom-web/studies/1.2.3/series/4.5/instances/6.7/metadata",
                Some(Viewed(study())),
            ),
            (Method::GET, "/dicom-web/studies/1.2.3", Some(Sent(study()))),
            (
                Method::GET,
                "/dicom-web/studies/1.2.3/series/4.5",
                Some(Sent(study())),
            ),
            (
                Method::GET,
                "/dicom-web/studies/1.2.3/series/4.5/instances/6.7",
                Some(Sent(study())),
            ),
            (
                Method::GET,
                "/dicom-web/studies/1.2.3/series/4.5/instances/6.7/frames/1,2",
                Some(Sent(study())),
            ),
            (
                Method::GET,
                "/dicom-web/studies/1.2.3/series/4.5/instances/6.7/rendered",
                Some(Sent(study())),
            ),
            (Method::DELETE, "/dicom-web/studies/1.2.3", None),
            (Method::GET, "/dicom-web", None),
            // a path that only starts like the root is the REST API's
            (Method::GET, "/dicom-webx/studies", None),
            // the REST API
            (Method::GET, "/studies", Some(Search)),
            (Method::GET, "/patients", Some(Search)),
            (Method::GET, "/series", Some(Search)),
            (Method::GET, "/instances", Some(Search)),
            (Method::POST, "/tools/find", Some(Search)),
            (Method::POST, "/instances", Some(Stores)),
            (
                Method::GET,
                "/studies/abc",
                Some(Viewed(rest(Level::Study, "abc"))),
            ),
            (
                Method::GET,
                "/studies/abc/series",
                Some(Viewed(rest(Level::Study, "abc"))),
            ),
            (
                Method::GET,
                "/studies/abc/statistics",
                Some(Viewed(rest(Level::Study, "abc"))),
            ),
            (
                Method::GET,
                "/patients/p1/studies",
                Some(Viewed(rest(Level::Patient, "p1"))),
            ),
            (
                Method::GET,
                "/series/s1/instances",
                Some(Viewed(rest(Level::Series, "s1"))),
            ),
            (
                Method::GET,
                "/instances/i1",
                Some(Viewed(rest(Level::Instance, "i1"))),
            ),
            (
                Method::GET,
                "/instances/i1/study",
                Some(Viewed(rest(Level::Instance, "i1"))),
            ),
            (
                Method::GET,
                "/instances/i1/file",
                Some(Sent(rest(Level::Instance, "i1"))),
            ),
            (
                Method::GET,
                "/studies/abc/archive",
                Some(Exported(rest(Level::Study, "abc"))),
            ),
            (
                Method::GET,
                "/patients/p1/archive",
                Some(Exported(rest(Level::Patient, "p1"))),
            ),
            (
                Method::GET,
                "/studies/abc/media",
                Some(Exported(rest(Level::Study, "abc"))),
            ),
            (
                Method::GET,
                "/tools/create-archive",
                Some(Exported(Target::Listed)),
            ),
            (
                Method::POST,
                "/tools/create-archive",
                Some(Exported(Target::Listed)),
            ),
            (
                Method::DELETE,
                "/studies/abc",
                Some(Deleted(Level::Study, "abc".into())),
            ),
            (
                Method::DELETE,
                "/patients/p1",
                Some(Deleted(Level::Patient, "p1".into())),
            ),
            (
                Method::DELETE,
                "/series/s1",
                Some(Deleted(Level::Series, "s1".into())),
            ),
            (
                Method::DELETE,
                "/instances/i1",
                Some(Deleted(Level::Instance, "i1".into())),
            ),
            (Method::DELETE, "/studies/abc/series", None),
            (Method::PUT, "/studies/abc", None),
            // the web interface's API
            (Method::GET, "/ui/api/studies", Some(Search)),
            (Method::GET, "/ui/api/patients", Some(Search)),
            (Method::GET, "/ui/api/studies/facets", Some(Search)),
            (
                Method::GET,
                "/ui/api/annotations/1.2.3",
                Some(Viewed(study())),
            ),
            (
                Method::PUT,
                "/ui/api/annotations/1.2.3",
                Some(Annotated(study())),
            ),
            (
                Method::PUT,
                "/studies/abc/labels/reviewed",
                Some(Annotated(rest(Level::Study, "abc"))),
            ),
            (
                Method::DELETE,
                "/series/abc/labels/reviewed",
                Some(Annotated(rest(Level::Series, "abc"))),
            ),
            (
                Method::PUT,
                "/patients/abc/metadata/note",
                Some(Annotated(rest(Level::Patient, "abc"))),
            ),
            (
                Method::DELETE,
                "/instances/abc/metadata/note",
                Some(Annotated(rest(Level::Instance, "abc"))),
            ),
            // not a deletion of the resource, and nothing else is written under one
            (Method::DELETE, "/studies/abc/labels", None),
            (Method::PUT, "/studies/abc/archive", None),
            (Method::GET, "/ui/api/protocols", None),
            // nothing of the archive's data
            (Method::GET, "/healthz", None),
            (Method::GET, "/ui/", None),
            (Method::GET, "/ui/config.json", None),
            (Method::GET, "/users", None),
            (Method::POST, "/auth/login", None),
            // remote AEs
            (
                Method::GET,
                "/modalities/a/studies",
                Some(RemoteSearch("a".into())),
            ),
            (
                Method::POST,
                "/modalities/a/move",
                Some(RemoteMove("a".into())),
            ),
            (
                Method::POST,
                "/modalities/a/store",
                Some(RemoteStore("a".into())),
            ),
            (Method::GET, "/modalities", None),
            (Method::POST, "/modalities/a/echo", None),
            (Method::GET, "/modalities/a/move", None),
            (Method::GET, "/jobs/j1", None),
            (Method::GET, "/", None),
        ];
        for (method, path, expected) in cases {
            assert_eq!(of(method.clone(), path), expected, "{method} {path}");
        }
        // another DICOMweb root
        assert_eq!(
            subject(&Method::GET, "/wado/studies", "/wado"),
            Some(Search)
        );
        assert_eq!(subject(&Method::GET, "/dicom-web/studies", "/wado"), None);
    }
}
