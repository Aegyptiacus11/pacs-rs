# Command-line tools

Six DIMSE tools come with pacs-rs, for a site that has no DCMTK at hand: to test a link, send
files, receive them, and search and pull from another archive. They are in the release archives
beside `pacs-server` (not in the Docker image), and are built from the source with
`cargo build --release -p pacs-dimse --features tools`. Each says what it takes with `--help`.

The remote AE is written `AE_TITLE@host:port`; `--aet` sets the tool's own AE title (`PACSRS`
unless given), `--timeout` the seconds to wait for the remote AE (30). A tool that fails, or
for which an instance failed, exits with status 1, having said why.

| Tool | Does |
|---|---|
| `echoscu HOSPITAL@10.0.0.9:104` | C-ECHO: whether the AE answers |
| `storescu HOSPITAL@10.0.0.9:104 study/ more.dcm` | C-STORE of files, and of every file under a folder, on one association; each file in its own transfer syntax when the AE takes it, decoded otherwise; a status a file |
| `storescp 11112 --output received/` | receives by C-STORE until stopped (Ctrl-C), as AE `STORESCP` (`--aet`), keeping each instance as sent: the files under `received/files`, an index of them beside |
| `findscu HOSPITAL@10.0.0.9:104 --patient-name 'DOE^*'` | C-FIND of studies, by `--patient-id`, `--patient-name`, `--accession-number`, `--study-date` (a date or a range) and `--modality`: one line of DICOM JSON a study |
| `movescu HOSPITAL@10.0.0.9:104 1.2.3... --destination STORESCP` | C-MOVE of studies to an AE the remote one knows |
| `getscu HOSPITAL@10.0.0.9:104 1.2.3... --output pulled/` | C-GET of studies into a folder, as `storescp` keeps them: the remote AE needs no address for this one |

`findscu`, `movescu` and `getscu` speak Study Root when the remote AE takes it and Patient Root
otherwise, as pacs-rs does ([DIMSE](dimse.md)); in Patient Root a move or a get names the
studies' patient, `--patient-id`. They do not speak TLS: for an AE reached over TLS, use
pacs-server's [remote archives](dimse.md).
