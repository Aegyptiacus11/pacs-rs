# REST API

At the server's root, compatible with the reference server's REST API (see [Comparing with a
reference server](building.md#comparing-with-a-reference-server)), so its clients work unchanged. Resources have public IDs: the SHA-1 of their
DICOM identifiers (`pacs_core::public_ids`), the same as the reference server gives them.

Where to read about it:

| | |
|---|---|
| [REST API, task by task](rest-guide.md) | sending, listing, fetching, searching and deleting, with the answers |
| [REST API reference](api-reference.md) | every request in full: parameters, answers, statuses, the permission it needs, and the objects. Written from the code |
| `/openapi.json` on a running server | the same description, as OpenAPI 3.1, for a client generator or an API browser |
| [Login](login.md) | how signing in, roles, tools and rights per project work; their requests are in the reference too |

What holds across the requests:

A patient is the studies that share its Patient ID. A deletion removes the index entries first,
then the files, so the index never names a missing file. A file that cannot be removed is
logged and left in the store, unindexed. Uploads wait while a deletion runs, so a file sent
again during a deletion is not lost. This holds within one server process: two pacs-rs
processes on one index do not coordinate yet.

**The [login](login.md) is off by default:** anyone who can reach the REST API can then delete.
Turn it on, or keep the server on a private network (see [Running](running.md)).

An instance sent again is kept as first stored (`AlreadyStored`) unless `PACS_OVERWRITE_INSTANCES`
is set. Lists of a study's series and a series' instances come in Series and Instance Number
order.

Not yet: labels, metadata, modifications; searching series or instances by anything but their UID.
