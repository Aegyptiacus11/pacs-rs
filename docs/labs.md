# Labs: learn by doing

Four practical sessions on one real CT study. You open a DICOM file byte by byte, store it in
a PACS, talk to that PACS the way a scanner does, and turn its numbers into a picture.

The labs follow the two pages before them, [DICOM: files and studies](dicom-files.md) and
[DICOM: the network](dicom-network.md). You can read first and practise after, or start here
and go back to the pages when a word is new.

| Lab | You will | Needs |
|---|---|---|
| [1. Inside a DICOM file](lab-file.md) | find the marker, the elements and the pixels of a file with a hex viewer, then with pydicom | the sample files |
| [2. Store and search](lab-archive.md) | send files to pacs-rs, watch it file them, search them, and add a second patient | the server |
| [3. DICOM on the network](lab-network.md) | echo, store, search and retrieve in the protocol scanners speak, and read three refusals | the server |
| [4. From numbers to a picture](lab-pixels.md) | check that compression lost nothing, read Hounsfield units, and window a slice | the server |

Each lab is a list of steps. A step gives a command, the answer you should get, and often a
question. Try the question before you open its answer.

## What you need

- A terminal on Linux or macOS, or WSL on Windows.
- [Docker](https://docs.docker.com/get-docker/), to run pacs-rs.
- `curl` and `xxd`, which Linux and macOS already have.
- [uv](https://docs.astral.sh/uv/getting-started/installation/), which runs the Python tools
  of the labs (pydicom, pynetdicom) without installing them.

No patient data is involved. The study is three slices of a public research CT, described in
[The sample study](sample-study.md).

## Set up once

Make a folder for the labs and download the server's Compose file and the three sample files
into it:

```bash
mkdir pacs-lab && cd pacs-lab
curl -fsSLO https://raw.githubusercontent.com/Aegyptiacus11/pacs-rs/main/compose.yaml
for n in 1 2 3; do
  curl -fsSLO "https://aegyptiacus11.github.io/pacs-rs/sample/0000$n.dcm"
done
```

The quickstart's server speaks HTTP only. Lab 3 also needs the DICOM network protocol, so add
a second file that turns it on. Docker Compose reads a `compose.override.yaml` beside
`compose.yaml` by itself:

```bash
cat > compose.override.yaml <<'EOF'
services:
  pacs:
    ports:
      - "127.0.0.1:11112:11112"
    environment:
      PACS_DIMSE_LISTEN: 0.0.0.0:11112
      PACS_DIMSE_REMOTES: lab=LAB@127.0.0.1:11113
EOF
```

The three settings say: publish port 11112 to this machine only, listen for DICOM on it, and
know one caller by the AE title `LAB`. Lab 3 explains each.

Start the server:

```bash
docker compose up -d --wait
```

Check that it answers. An empty list means a running, empty PACS:

```bash
curl http://127.0.0.1:8080/studies
```

```json
[]
```

## Start again from empty

Each lab says at its top what the server must hold. To empty the server at any time and
start again:

```bash
docker compose down -v
docker compose up -d --wait
```

When you have finished, `docker compose down -v` removes the server's data with it.

## How these labs were checked

Every command of the four labs was run, in order, against pacs-rs 0.16.3, and the answers
printed on the pages are the ones it gave. Unlike the quickstart, the labs are not yet run
again automatically when the documentation changes, because they use tools outside the
server (pydicom, pynetdicom). If an answer differs on your machine,
[open an issue](https://github.com/Aegyptiacus11/pacs-rs/issues).

## For teachers

- **One session or four.** The labs are independent pages, in the order a course would take
  them. No lab has been timed with a class yet, so plan the first session with room to spare.
- **Before the session**, have each machine run the set-up above once. It downloads the
  server's image and the Python tools, which is the only part that needs a good connection.
- **Nothing is shared.** Each student runs their own server on their own machine, reachable
  from that machine only, so nobody can break a neighbour's work.
- **The questions have answers on the page**, folded. They are meant for discussion, not for
  grading.
