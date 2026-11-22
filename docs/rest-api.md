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
then moves the files to the [trash](trash.md) (or removes them, with `PACS_TRASH_DAYS=0`), so
the index never names a missing file. A file that cannot be moved or removed is logged and
left in the store, unindexed. Each stored file has a key of its own
(its UIDs and a random part), so a deletion removes only the files it took out of the index,
never one an upload is storing meanwhile; and the index decides inside one transaction whether
an upload adds, replaces (the replaced file then goes) or finds the instance there already.
This holds across [several servers on one Postgres index](running.md#several-servers).

**The [login](login.md) is off by default:** anyone who can reach the REST API can then delete.
Turn it on, or keep the server on a private network (see [Running](running.md)).

An instance sent again is kept as first stored (`AlreadyStored`) unless `PACS_OVERWRITE_INSTANCES`
is set. Lists of a study's series and a series' instances come in Series and Instance Number
order.

**Labels** are free words put on a patient, study, series or instance to group and find them
(`PUT`, `GET` and `DELETE` on `/{level}/{id}/labels/{label}`, `GET /{level}/{id}/labels`,
`GET /tools/labels`): 1 to 64 ASCII letters, digits, `_` and `-`. Each resource's answer has
its `Labels`, and `/tools/find` keeps the resources with all, any or none of those it names
(`Labels`, `LabelsConstraint`). Putting and taking them off needs `write`; a deletion takes
the labels of what it removed.

**Metadata** are values under names on a patient, study, series or instance
(`GET /{level}/{id}/metadata`, `?expand` for the values; `GET`, `PUT` with the value as the body,
and `DELETE` on `/{level}/{id}/metadata/{name}`). The server's own are read only: `LastUpdate`
at every level, and of an instance `IndexInSeries`, `ReceptionDate`, `SopClassUid` and
`TransferSyntax`. A user's name is 1 to 64 ASCII letters, digits, `_` and `-`, its value UTF-8
text of at most 64 KiB. Setting and removing them needs `write`; a deletion takes the
metadata of what it removed.

Not yet: searching series or instances by anything but their UID or their labels. Not
planned: modifications (`/modify`).
