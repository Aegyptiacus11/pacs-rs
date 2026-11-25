# Lab 3: DICOM on the network

You will talk to pacs-rs in the protocol a scanner speaks: ask whether it is there, send it
an image, search it and pull a study back. Three times it will refuse you, and each refusal
teaches a rule of the protocol.

- **Before you start:** the [set-up](labs.md#set-up-once) of the labs, and an empty server:
  [start again from empty](labs.md#start-again-from-empty) if you have just done Lab 2.
- **Read with it:** [DICOM: the network](dicom-network.md).

The commands use the tools of [pynetdicom](https://pydicom.github.io/pynetdicom/), a Python
library for this protocol. `uvx` runs them without installing anything. To type less, define
a shortcut for this terminal:

```bash
dcm() { uvx --from pynetdicom python -m pynetdicom "$@"; }
```

`dcm echoscu …` now runs pynetdicom's `echoscu`. The hospital world's usual tools, DCMTK's
`echoscu`, `storescu`, `findscu` and `getscu`, take nearly the same options.

Recall the [set-up](labs.md#set-up-once): the server listens for DICOM on port 11112, its AE
title is `PACSRS`, and it knows one caller, whose AE title is `LAB`.

## 1. Are you there? First refusal

C-ECHO is the protocol's ping. Send one:

```bash
dcm echoscu 127.0.0.1 11112 -v
```

```text
I: Requesting Association
E: Association Rejected
E: Result: Rejected Permanent, Source: Service User
E: Reason: Called AE title not recognised
```

The server is there, and it refused. The tool called it by a default name, `ANY-SCP`, and no
application entity of that name lives on this port.

Call it by its name. `-aec` sets the *called* AE title:

```bash
dcm echoscu 127.0.0.1 11112 -aec PACSRS -v
```

```text
I: Requesting Association
I: Association Accepted
I: Sending Echo Request: MsgID 1
I: Received Echo Response (Status: 0x0000 - Success)
I: Releasing Association
```

Read the five lines as a conversation: the association is asked for and accepted, one message
goes out and its answer comes back with status `0x0000`, and the association is released.
Every exchange in this lab has that shape.

**Question.** The first attempt failed before any echo was sent. At which step, and what
would have been worse about failing later?

<details>
<summary>Answer</summary>

At the association, the agreement that comes before any message. Failing there is cheap and
clear: nothing was sent, and the reason says which side must change. A scanner that found out
only after sending 400 images that it was talking to the wrong system would have wasted the
transfer, and the images might be somewhere they should not be.

</details>

## 2. Send an image as a scanner does

The server is empty. Send it the three slices with C-STORE, one association per file:

```bash
for f in 00001.dcm 00002.dcm 00003.dcm; do
  dcm storescu 127.0.0.1 11112 "$f" -aec PACSRS -v
done
```

For each file:

```text
I: Requesting Association
I: Association Accepted
I: Sending file: 00001.dcm
I: Sending Store Request: MsgID 1, (CT)
I: Received Store Response (Status: 0x0000 - Success)
I: Releasing Association
```

Check over HTTP that they arrived:

```bash
curl "http://127.0.0.1:8080/dicom-web/studies?PatientID=LITS-0"
```

In the answer, `00201208` counts three instances:

```json
"00201208": {"vr": "IS", "Value": [3]}
```

One archive, two doors. A file that comes in over the DICOM protocol and one that comes in
over HTTP go through the same checks and end in the same study, under the same identifiers
as in [Lab 2](lab-archive.md). Send a file a second time and the store is answered Success
again, with the first copy kept.

## 3. Search. Second refusal

C-FIND asks which studies match. `-S` chooses the study-level way of asking, each `-k` is one
key: a key with a value is a condition, a key without one asks for that element back.

```bash
dcm findscu 127.0.0.1 11112 -aec PACSRS -S \
  -k QueryRetrieveLevel=STUDY -k PatientID=LITS-0 \
  -k StudyInstanceUID -k StudyDescription
```

```text
I: Requesting Association
I: Association Accepted
I: Sending Find Request: MsgID 1
I: 
I: # Request Identifier
I: (0008,0052) CS [STUDY]                                  # 1 QueryRetrieveLevel
I: (0008,1030) LO (no value available)                     # 0 StudyDescription
I: (0010,0020) LO [LITS-0]                                 # 1 PatientID
I: (0020,000D) UI (no value available)                     # 0 StudyInstanceUID
I: 
I: Find SCP Result: 0x0124 (Failure)
I: Releasing Association
```

This time the association was accepted and the *message* was refused: status `0x0124` is
*Refused: Not authorized*. pacs-rs lets anyone who knows its AE title echo and store, and
answers searches only for the callers it has been told about.

You are one of them. The set-up listed a caller named `LAB`. Say who you are with `-aet`,
the *calling* AE title, and ask for the number of instances too:

```bash
dcm findscu 127.0.0.1 11112 -aet LAB -aec PACSRS -S \
  -k QueryRetrieveLevel=STUDY -k PatientID=LITS-0 \
  -k StudyInstanceUID -k StudyDescription -k NumberOfStudyRelatedInstances
```

```text
I: Find SCP Response: 1 - 0xFF00 (Pending)
I: 
I: # Response Identifier
I: (0008,0052) CS [STUDY]                                  # 1 QueryRetrieveLevel
I: (0008,0054) AE [PACSRS]                                 # 1 RetrieveAETitle
I: (0008,1030) LO [Three slices of LiTS volume 0]          # 1 StudyDescription
I: (0010,0020) LO [LITS-0]                                 # 1 PatientID
I: (0020,000D) UI [2.25.3100000]                           # 1 StudyInstanceUID
I: (0020,1208) IS [3]                                      # 1 NumberOfStudyRelatedInstances
I: 
I: Find SCP Result: 0x0000 (Success)
I: Releasing Association
```

The lines before these repeat your request. Then comes one response per match, each with the
status *Pending*, which means "here is one, more may follow", and a last one with *Success*,
which means "that was all".

**Questions.**

1. Compare this answer with the DICOMweb search of step 2. What is the same?
2. You became `LAB` by typing three letters. What does that say about AE titles?

<details>
<summary>Answers</summary>

1. The elements. Both answers describe the study with the same tags and values:
   `(0020,000D)` here is `"0020000D"` there. C-FIND and QIDO-RS are two ways of carrying one
   question, and in pacs-rs the same code matches both.
2. An AE title is a name, not a password. Anyone can claim one, so the list of known callers
   keeps out systems that were never set up, not people who lie. Real protection comes from
   the network (who can reach the port at all) or from certificates
   ([DIMSE over TLS](dimse.md#dimse-over-tls)).

</details>

## 4. Go down a level

A search is asked level by level: studies, then the series of one study, then the instances
of one series. List the instances of the sample's series:

```bash
dcm findscu 127.0.0.1 11112 -aet LAB -aec PACSRS -S \
  -k QueryRetrieveLevel=IMAGE -k StudyInstanceUID=2.25.3100000 \
  -k SeriesInstanceUID=2.25.3200000 -k SOPInstanceUID -k InstanceNumber
```

You get three *Pending* responses, one per instance, with the SOP Instance UIDs
`2.25.3300001` to `2.25.3300003`, then *Success*.

## 5. Pull the study back

C-GET asks the archive to send a study back on the association that asks. `-od` is the
folder to write into:

```bash
mkdir received
dcm getscu 127.0.0.1 11112 -aet LAB -aec PACSRS -S -od received \
  -k QueryRetrieveLevel=STUDY -k StudyInstanceUID=2.25.3100000
```

```text
I: Requesting Association
I: Association Accepted
I: Sending Get Request: MsgID 1
I: 
I: # Request Identifier
I: (0008,0052) CS [STUDY]                                  # 1 QueryRetrieveLevel
I: (0020,000D) UI [2.25.3100000]                           # 1 StudyInstanceUID
I: 
I: Received Store Request
I: Storing DICOM file: CT.2.25.3300001
I: Get SCP Response: 1 - 0xFF00 (Pending)
I: Sub-Operations Remaining: 2, Completed: 1, Failed: 0, Warning: 0
I: Received Store Request
I: Storing DICOM file: CT.2.25.3300002
I: Get SCP Response: 2 - 0xFF00 (Pending)
I: Sub-Operations Remaining: 1, Completed: 2, Failed: 0, Warning: 0
I: Received Store Request
I: Storing DICOM file: CT.2.25.3300003
I: Get SCP Result: 0x0000 (Success)
I: Sub-Operations Remaining: 0, Completed: 3, Failed: 0, Warning: 0
I: Releasing Association
```

Look at who stores. You asked, and then *the server* sent three Store Requests to *you*: for
the length of this association the roles turned around, and your tool was the storage
provider. The folder `received` now holds three files.

**Question.** Print the transfer syntax of a received file with
`uvx pydicom show received/CT.2.25.3300001 | grep "Transfer Syntax"`. It says
`Implicit VR Little Endian`. You sent Explicit VR Little Endian. Is the image damaged?

<details>
<summary>Answer</summary>

No. The transfer syntax is how the bytes are written, not what they say. When the association
opened, your tool and the server agreed on a transfer syntax for CT images, and the server
wrote the images in it. The elements and the pixel values are the ones you sent;
[Lab 4](lab-pixels.md) checks that kind of claim value by value.

</details>

## 6. Send it elsewhere. Third refusal

C-MOVE is the older way to pull. It does not send to whoever asks: it names a destination by
AE title, and the archive opens a new association to that destination. Ask for the study to
be sent to an AE title the server has never heard of:

```bash
dcm movescu 127.0.0.1 11112 -aet LAB -aec PACSRS -aem ELSEWHERE -S \
  -k QueryRetrieveLevel=STUDY -k StudyInstanceUID=2.25.3100000
```

```text
I: Requesting Association
I: Association Accepted
I: Sending Move Request: MsgID 1
I: 
I: # Request Identifier
I: (0008,0052) CS [STUDY]                                  # 1 QueryRetrieveLevel
I: (0020,000D) UI [2.25.3100000]                           # 1 StudyInstanceUID
I: 
I: Move SCP Result: 0xA801 (Failure)
I: Sub-Operations Remaining: 0, Completed: 0, Failed: 0, Warning: 0
I: Releasing Association
```

Status `0xA801` is *Refused: Move Destination unknown*. Nothing was sent.

**Question.** An AE title is a short name. What would the server need, besides the name
`ELSEWHERE`, to send the study there? And why is it right to refuse without it?

<details>
<summary>Answer</summary>

A host and a port: the request carries only the name. The archive's administrator must have
registered where `ELSEWHERE` is, and in pacs-rs must also have allowed studies to be sent
there (`PACS_DIMSE_MOVE_DESTINATIONS`). That is by design. If a request could name any
address, anyone able to reach the archive could have patients' studies sent anywhere. It is
also why C-MOVE is the classic cause of "it does not work" between two hospital systems, and
why C-GET exists: it needs nothing set up on the archive's side.

</details>

## 7. Read the server's side

The server logged every one of these conversations. Look at the last lines:

```bash
docker compose logs --tail 20 pacs
```

Find your C-MOVE: a line with `C-MOVE`, `calling="LAB"`, `destination=ELSEWHERE` and
`status=A801H (Failure)`. In a hospital this log, and the [audit trail](audit.md) beside it,
is where an administrator looks when a scanner "cannot send".

## What you learned

- Two DICOM systems open an association and agree on what they will do before any message is
  sent. A wrong AE title fails there.
- C-ECHO tests the link, C-STORE sends an instance, C-FIND searches, C-GET and C-MOVE
  retrieve.
- Every message is answered with a status: `0000` Success, `FF00` Pending, and a code that
  names the reason when it is refused.
- An AE title is a name, not a password.
- C-MOVE sends to a third party the archive must already know. C-GET sends back to whoever
  asks.

Next: [Lab 4: from numbers to a picture](lab-pixels.md).
