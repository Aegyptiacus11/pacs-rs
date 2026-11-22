--
-- PostgreSQL database dump
--


-- Dumped from database version 18.6
-- Dumped by pg_dump version 18.6

SET statement_timeout = 0;
SET lock_timeout = 0;
SET idle_in_transaction_session_timeout = 0;
SET transaction_timeout = 0;
SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;
SELECT pg_catalog.set_config('search_path', '', false);
SET check_function_bodies = false;
SET xmloption = content;
SET client_min_messages = warning;
SET row_security = off;

SET default_tablespace = '';

SET default_table_access_method = heap;

--
-- Name: _sqlx_migrations; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public._sqlx_migrations (
    version bigint NOT NULL,
    description text NOT NULL,
    installed_on timestamp with time zone DEFAULT now() NOT NULL,
    success boolean NOT NULL,
    checksum bytea NOT NULL,
    execution_time bigint NOT NULL
);


--
-- Name: account_projects; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.account_projects (
    account text NOT NULL,
    project text NOT NULL
);


--
-- Name: auth_refresh_families; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.auth_refresh_families (
    id text NOT NULL,
    user_id text NOT NULL,
    created_at bigint NOT NULL,
    revoked_at bigint
);


--
-- Name: auth_refresh_tokens; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.auth_refresh_tokens (
    token_hash text NOT NULL,
    family_id text NOT NULL,
    issued_at bigint NOT NULL,
    expires_at bigint NOT NULL,
    used_at bigint
);


--
-- Name: clients; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.clients (
    id text NOT NULL,
    name text NOT NULL,
    secret_hash text NOT NULL,
    role text NOT NULL,
    created_by text NOT NULL,
    created_ms bigint NOT NULL,
    last_token_ms bigint
);


--
-- Name: instances; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.instances (
    sop_instance_uid text NOT NULL,
    series_instance_uid text NOT NULL,
    sop_class_uid text NOT NULL,
    instance_number bigint,
    transfer_syntax_uid text NOT NULL,
    object_key text NOT NULL,
    size_bytes bigint NOT NULL,
    attributes text NOT NULL,
    metadata text NOT NULL,
    public_id text,
    main_tags text,
    received_ms bigint NOT NULL,
    sha256 text,
    checked_ms bigint,
    check_failure text
);


--
-- Name: jobs; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.jobs (
    id bigint NOT NULL,
    kind text NOT NULL,
    remote text NOT NULL,
    destination text NOT NULL,
    studies text NOT NULL,
    state text NOT NULL,
    progress text NOT NULL,
    failed_instances text NOT NULL,
    error text,
    cancel_asked bigint NOT NULL,
    restarts bigint NOT NULL,
    started_ms bigint NOT NULL,
    finished_ms bigint,
    heartbeat_ms bigint
);


--
-- Name: jobs_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.jobs_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: jobs_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.jobs_id_seq OWNED BY public.jobs.id;


--
-- Name: limited_accounts; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.limited_accounts (
    account text NOT NULL
);


--
-- Name: roles; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.roles (
    id text NOT NULL,
    name text NOT NULL,
    permissions text NOT NULL,
    created_ms bigint NOT NULL
);


--
-- Name: series; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.series (
    series_instance_uid text NOT NULL,
    study_instance_uid text NOT NULL,
    modality text,
    series_number bigint,
    attributes text NOT NULL,
    public_id text,
    main_tags text
);


--
-- Name: studies; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.studies (
    study_instance_uid text NOT NULL,
    patient_id text,
    patient_name text,
    study_date text,
    accession_number text,
    attributes text NOT NULL,
    public_id text,
    patient_public_id text,
    main_tags text,
    patient_main_tags text,
    project text
);


--
-- Name: trash; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.trash (
    id bigint NOT NULL,
    deleted_ms bigint NOT NULL,
    level text NOT NULL,
    public_id text NOT NULL,
    studies text NOT NULL
);


--
-- Name: trash_files; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.trash_files (
    trash_key text NOT NULL,
    trash_id bigint NOT NULL,
    object_key text NOT NULL
);


--
-- Name: trash_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

CREATE SEQUENCE public.trash_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1;


--
-- Name: trash_id_seq; Type: SEQUENCE OWNED BY; Schema: public; Owner: -
--

ALTER SEQUENCE public.trash_id_seq OWNED BY public.trash.id;


--
-- Name: ui_annotations; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.ui_annotations (
    study_instance_uid text NOT NULL,
    body text NOT NULL,
    updated_ms bigint NOT NULL
);


--
-- Name: ui_protocols; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.ui_protocols (
    id text NOT NULL,
    body text NOT NULL,
    updated_ms bigint NOT NULL
);


--
-- Name: user_identities; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.user_identities (
    issuer text NOT NULL,
    subject text NOT NULL,
    user_id text NOT NULL
);


--
-- Name: users; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.users (
    id text NOT NULL,
    login text NOT NULL,
    password_hash text NOT NULL,
    role text NOT NULL,
    active bigint NOT NULL,
    created_ms bigint NOT NULL
);


--
-- Name: jobs id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.jobs ALTER COLUMN id SET DEFAULT nextval('public.jobs_id_seq'::regclass);


--
-- Name: trash id; Type: DEFAULT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.trash ALTER COLUMN id SET DEFAULT nextval('public.trash_id_seq'::regclass);


--
-- Data for Name: _sqlx_migrations; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public._sqlx_migrations VALUES (1, 'hierarchy', '2026-11-22 17:19:07.633465+00', true, '\x55c9bff9c2bd52d7d21620e61f82527507d1b28916e89082c1e21e8ff5d82ba38887bd61898fdf90083c28bddb1485d5', 10126467);
INSERT INTO public._sqlx_migrations VALUES (2, 'ui protocols', '2026-11-22 17:19:07.644349+00', true, '\x85735819bcdf21efb52209c4feb5ca4e97bb2930dfdb9f78bd4350a5d5dd03c7696e72731249956ec21750db5bc4d66b', 2182756);
INSERT INTO public._sqlx_migrations VALUES (3, 'ui annotations', '2026-11-22 17:19:07.647015+00', true, '\x18dc1a3f0c95806b8ee0170ff250912611c422b5baead85c84c9989cbb0324aecb06d59ea483d12d68a26ba6aaea0c47', 2062979);
INSERT INTO public._sqlx_migrations VALUES (4, 'users', '2026-11-22 17:19:07.649606+00', true, '\xd21d88444d6080a6f5019451b1966c8d4c5251ba60b716b67e7b449a3b6a6d5d834b82ecc3230f4bd1e4b4c08b371267', 7116544);
INSERT INTO public._sqlx_migrations VALUES (5, 'roles', '2026-11-22 17:19:07.657385+00', true, '\x110bce285d9ffa5a6aaea0c079107995d756c442a8174f5b175fa94ecfb75e0e01cbb358d88882fa318ca03308e1e4a7', 2244152);
INSERT INTO public._sqlx_migrations VALUES (6, 'clients', '2026-11-22 17:19:07.660105+00', true, '\xa8817e19316b59f7e7a178a2f9db2cc8041d66df5840adc237f346f0ed2087e3076db216e8128f11cd1dc98f4a08391b', 2331368);
INSERT INTO public._sqlx_migrations VALUES (7, 'study project', '2026-11-22 17:19:07.662972+00', true, '\x908913e9d66cb251d06c395224e9790ece7536156a25a62cc9d9a75665060e5b0a0336688dc3ab5aaf6b3524cb3e969d', 1206066);
INSERT INTO public._sqlx_migrations VALUES (8, 'account projects', '2026-11-22 17:19:07.664693+00', true, '\x2fa4c77472eeb85a024c8d6f706a7c0e5a0c0fdda4a1ee9c2de8fee3f3d1583ed781b2788d69ca22609279e236df2887', 3247603);
INSERT INTO public._sqlx_migrations VALUES (9, 'user identities', '2026-11-22 17:19:07.6684+00', true, '\xdc5f4d6b9fce65a97c5248d18a718e68fd7a1ce8533d50b37b9c8479279eb3919c16c9e0b6e13d9ea31e125f81377116', 2888934);
INSERT INTO public._sqlx_migrations VALUES (10, 'file checks', '2026-11-22 17:19:07.671801+00', true, '\x57c1ba094ed913598f98b7b93d2d1d18c64575ea4fab13d7659d9261bc6fcc5e191424d6391f5126c853698c7e74eeb7', 1497518);
INSERT INTO public._sqlx_migrations VALUES (11, 'jobs', '2026-11-22 17:19:07.673807+00', true, '\xbdeaab50591d9ae170c6f718a348645d7c63970836e907357a933d7b61bc9ceb649070355e5e48f6cecebc41d604af5e', 3303108);
INSERT INTO public._sqlx_migrations VALUES (12, 'trash', '2026-11-22 17:19:07.677768+00', true, '\x25065935e8192deda67da87b8e882baedb5038054565d35f3a4bda8b875c4966d9c122fde3c23d9705c28b2c6c52f8be', 5207357);
INSERT INTO public._sqlx_migrations VALUES (13, 'job heartbeats', '2026-11-22 17:19:07.683636+00', true, '\xd65615f2ddb398ac9e410e8778c0f4592011148e021cd529f54b1f486dd0b976d49bbd106896813b97927d36412dd2ac', 1118439);


--
-- Data for Name: account_projects; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: auth_refresh_families; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: auth_refresh_tokens; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: clients; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: instances; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.instances VALUES ('2.25.3300001', '2.25.3200000', '1.2.840.10008.5.1.4.1.1.2', 1, '1.2.840.10008.1.2.4.80', '2.25.3100000/2.25.3200000/2.25.3300001.1e7ae74febebdc05.dcm', 146666, '{"00080016":{"vr":"UI","Value":["1.2.840.10008.5.1.4.1.1.2"]},"00080018":{"vr":"UI","Value":["2.25.3300001"]},"00200013":{"vr":"IS","Value":[1]},"00280010":{"vr":"US","Value":[512]},"00280011":{"vr":"US","Value":[512]},"00280100":{"vr":"US","Value":[16]}}', '{"00080008":{"vr":"CS","Value":["DERIVED","SECONDARY","AXIAL"]},"00080016":{"vr":"UI","Value":["1.2.840.10008.5.1.4.1.1.2"]},"00080018":{"vr":"UI","Value":["2.25.3300001"]},"00080020":{"vr":"DA","Value":["20261117"]},"00080030":{"vr":"TM","Value":["120000"]},"00080050":{"vr":"SH"},"00080060":{"vr":"CS","Value":["CT"]},"00080090":{"vr":"PN"},"00081030":{"vr":"LO","Value":["Three slices of LiTS volume 0"]},"0008103E":{"vr":"LO","Value":["Axial 5 mm"]},"00100010":{"vr":"PN","Value":[{"Alphabetic":"VOLUME 0^LITS"}]},"00100020":{"vr":"LO","Value":["LITS-0"]},"00100030":{"vr":"DA"},"00100040":{"vr":"CS"},"00180050":{"vr":"DS","Value":["5"]},"00185100":{"vr":"CS","Value":["HFS"]},"0020000D":{"vr":"UI","Value":["2.25.3100000"]},"0020000E":{"vr":"UI","Value":["2.25.3200000"]},"00200010":{"vr":"SH","Value":["1"]},"00200011":{"vr":"IS","Value":[1]},"00200013":{"vr":"IS","Value":[1]},"00200032":{"vr":"DS","Value":["-172.899994","-180","-48"]},"00200037":{"vr":"DS","Value":["1","0","0","0","1","0"]},"00200052":{"vr":"UI","Value":["2.25.3200000.1"]},"00201041":{"vr":"DS","Value":["-48"]},"00280002":{"vr":"US","Value":[1]},"00280004":{"vr":"CS","Value":["MONOCHROME2"]},"00280010":{"vr":"US","Value":[512]},"00280011":{"vr":"US","Value":[512]},"00280030":{"vr":"DS","Value":["0.703125","0.703125"]},"00280100":{"vr":"US","Value":[16]},"00280101":{"vr":"US","Value":[16]},"00280102":{"vr":"US","Value":[15]},"00280103":{"vr":"US","Value":[1]},"00281050":{"vr":"DS","Value":["40"]},"00281051":{"vr":"DS","Value":["400"]},"00281052":{"vr":"DS","Value":["0"]},"00281053":{"vr":"DS","Value":["1"]},"00281054":{"vr":"LO","Value":["HU"]}}', '518d98a6-f5a3181d-2984303f-271f3d0d-c973dfc2', '{"SOPInstanceUID":"2.25.3300001","InstanceNumber":"1","ImagePositionPatient":"-172.899994\\-180\\-48","ImageOrientationPatient":"1\\0\\0\\0\\1\\0"}', 1795367948659, '9a63bec1547bce1b443e9d8be009aa16b0274228beea359ea369cec3c97341f5', 1795367948659, NULL);
INSERT INTO public.instances VALUES ('2.25.3300002', '2.25.3200000', '1.2.840.10008.5.1.4.1.1.2', 2, '1.2.840.10008.1.2.4.80', '2.25.3100000/2.25.3200000/2.25.3300002.a0bf905e4b79a84f.dcm', 145784, '{"00080016":{"vr":"UI","Value":["1.2.840.10008.5.1.4.1.1.2"]},"00080018":{"vr":"UI","Value":["2.25.3300002"]},"00200013":{"vr":"IS","Value":[2]},"00280010":{"vr":"US","Value":[512]},"00280011":{"vr":"US","Value":[512]},"00280100":{"vr":"US","Value":[16]}}', '{"00080008":{"vr":"CS","Value":["DERIVED","SECONDARY","AXIAL"]},"00080016":{"vr":"UI","Value":["1.2.840.10008.5.1.4.1.1.2"]},"00080018":{"vr":"UI","Value":["2.25.3300002"]},"00080020":{"vr":"DA","Value":["20261117"]},"00080030":{"vr":"TM","Value":["120000"]},"00080050":{"vr":"SH"},"00080060":{"vr":"CS","Value":["CT"]},"00080090":{"vr":"PN"},"00081030":{"vr":"LO","Value":["Three slices of LiTS volume 0"]},"0008103E":{"vr":"LO","Value":["Axial 5 mm"]},"00100010":{"vr":"PN","Value":[{"Alphabetic":"VOLUME 0^LITS"}]},"00100020":{"vr":"LO","Value":["LITS-0"]},"00100030":{"vr":"DA"},"00100040":{"vr":"CS"},"00180050":{"vr":"DS","Value":["5"]},"00185100":{"vr":"CS","Value":["HFS"]},"0020000D":{"vr":"UI","Value":["2.25.3100000"]},"0020000E":{"vr":"UI","Value":["2.25.3200000"]},"00200010":{"vr":"SH","Value":["1"]},"00200011":{"vr":"IS","Value":[1]},"00200013":{"vr":"IS","Value":[2]},"00200032":{"vr":"DS","Value":["-172.899994","-180","-43"]},"00200037":{"vr":"DS","Value":["1","0","0","0","1","0"]},"00200052":{"vr":"UI","Value":["2.25.3200000.1"]},"00201041":{"vr":"DS","Value":["-43"]},"00280002":{"vr":"US","Value":[1]},"00280004":{"vr":"CS","Value":["MONOCHROME2"]},"00280010":{"vr":"US","Value":[512]},"00280011":{"vr":"US","Value":[512]},"00280030":{"vr":"DS","Value":["0.703125","0.703125"]},"00280100":{"vr":"US","Value":[16]},"00280101":{"vr":"US","Value":[16]},"00280102":{"vr":"US","Value":[15]},"00280103":{"vr":"US","Value":[1]},"00281050":{"vr":"DS","Value":["40"]},"00281051":{"vr":"DS","Value":["400"]},"00281052":{"vr":"DS","Value":["0"]},"00281053":{"vr":"DS","Value":["1"]},"00281054":{"vr":"LO","Value":["HU"]}}', '8a3affa1-2acb2dfe-012c8fcf-a1d113d6-bdf4a1fb', '{"SOPInstanceUID":"2.25.3300002","InstanceNumber":"2","ImagePositionPatient":"-172.899994\\-180\\-43","ImageOrientationPatient":"1\\0\\0\\0\\1\\0"}', 1795367948682, '122656b66f2f0cc5be5f7b7b513ff88b32bc776d988760af987af6f895bedd30', 1795367948682, NULL);
INSERT INTO public.instances VALUES ('2.25.3300003', '2.25.3200000', '1.2.840.10008.5.1.4.1.1.2', 3, '1.2.840.10008.1.2.4.80', '2.25.3100000/2.25.3200000/2.25.3300003.5b39a2fe449c63db.dcm', 143670, '{"00080016":{"vr":"UI","Value":["1.2.840.10008.5.1.4.1.1.2"]},"00080018":{"vr":"UI","Value":["2.25.3300003"]},"00200013":{"vr":"IS","Value":[3]},"00280010":{"vr":"US","Value":[512]},"00280011":{"vr":"US","Value":[512]},"00280100":{"vr":"US","Value":[16]}}', '{"00080008":{"vr":"CS","Value":["DERIVED","SECONDARY","AXIAL"]},"00080016":{"vr":"UI","Value":["1.2.840.10008.5.1.4.1.1.2"]},"00080018":{"vr":"UI","Value":["2.25.3300003"]},"00080020":{"vr":"DA","Value":["20261117"]},"00080030":{"vr":"TM","Value":["120000"]},"00080050":{"vr":"SH"},"00080060":{"vr":"CS","Value":["CT"]},"00080090":{"vr":"PN"},"00081030":{"vr":"LO","Value":["Three slices of LiTS volume 0"]},"0008103E":{"vr":"LO","Value":["Axial 5 mm"]},"00100010":{"vr":"PN","Value":[{"Alphabetic":"VOLUME 0^LITS"}]},"00100020":{"vr":"LO","Value":["LITS-0"]},"00100030":{"vr":"DA"},"00100040":{"vr":"CS"},"00180050":{"vr":"DS","Value":["5"]},"00185100":{"vr":"CS","Value":["HFS"]},"0020000D":{"vr":"UI","Value":["2.25.3100000"]},"0020000E":{"vr":"UI","Value":["2.25.3200000"]},"00200010":{"vr":"SH","Value":["1"]},"00200011":{"vr":"IS","Value":[1]},"00200013":{"vr":"IS","Value":[3]},"00200032":{"vr":"DS","Value":["-172.899994","-180","-38"]},"00200037":{"vr":"DS","Value":["1","0","0","0","1","0"]},"00200052":{"vr":"UI","Value":["2.25.3200000.1"]},"00201041":{"vr":"DS","Value":["-38"]},"00280002":{"vr":"US","Value":[1]},"00280004":{"vr":"CS","Value":["MONOCHROME2"]},"00280010":{"vr":"US","Value":[512]},"00280011":{"vr":"US","Value":[512]},"00280030":{"vr":"DS","Value":["0.703125","0.703125"]},"00280100":{"vr":"US","Value":[16]},"00280101":{"vr":"US","Value":[16]},"00280102":{"vr":"US","Value":[15]},"00280103":{"vr":"US","Value":[1]},"00281050":{"vr":"DS","Value":["40"]},"00281051":{"vr":"DS","Value":["400"]},"00281052":{"vr":"DS","Value":["0"]},"00281053":{"vr":"DS","Value":["1"]},"00281054":{"vr":"LO","Value":["HU"]}}', '9cd16b73-7d0f8d85-241e9bbc-f9ddb0a0-afb3d988', '{"SOPInstanceUID":"2.25.3300003","InstanceNumber":"3","ImagePositionPatient":"-172.899994\\-180\\-38","ImageOrientationPatient":"1\\0\\0\\0\\1\\0"}', 1795367948699, 'aea3d4d8b7da963d8614f3217cc38547c8d61a67d51321e195cc65b5fcc8aff4', 1795367948699, NULL);


--
-- Data for Name: jobs; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: limited_accounts; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: roles; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.roles VALUES ('reader', 'Reader', 'read', 0);
INSERT INTO public.roles VALUES ('editor', 'Editor', 'read,write,remote', 1);
INSERT INTO public.roles VALUES ('manager', 'Manager', 'read,write,remote,delete', 2);
INSERT INTO public.roles VALUES ('admin', 'Admin', 'read,write,remote,delete,users', 3);


--
-- Data for Name: series; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.series VALUES ('2.25.3200000', '2.25.3100000', 'CT', 1, '{"00080060":{"vr":"CS","Value":["CT"]},"0008103E":{"vr":"LO","Value":["Axial 5 mm"]},"0020000E":{"vr":"UI","Value":["2.25.3200000"]},"00200011":{"vr":"IS","Value":[1]}}', '95204644-8052ee6c-ad3a3db9-22191e1f-3a6aeb05', '{"Modality":"CT","SeriesDescription":"Axial 5 mm","SeriesInstanceUID":"2.25.3200000","SeriesNumber":"1","ImageOrientationPatient":"1\\0\\0\\0\\1\\0"}');


--
-- Data for Name: studies; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.studies VALUES ('2.25.3100000', 'LITS-0', 'VOLUME 0^LITS', '20261117', NULL, '{"00080020":{"vr":"DA","Value":["20261117"]},"00080030":{"vr":"TM","Value":["120000"]},"00080050":{"vr":"SH"},"00080090":{"vr":"PN"},"00081030":{"vr":"LO","Value":["Three slices of LiTS volume 0"]},"00100010":{"vr":"PN","Value":[{"Alphabetic":"VOLUME 0^LITS"}]},"00100020":{"vr":"LO","Value":["LITS-0"]},"00100030":{"vr":"DA"},"00100040":{"vr":"CS"},"0020000D":{"vr":"UI","Value":["2.25.3100000"]},"00200010":{"vr":"SH","Value":["1"]}}', '478cde9c-3f36815e-7253a722-f52d98c8-d15779eb', '64c94bda-110c6fa1-720a3d46-248247e2-bdf1f055', '{"StudyDate":"20261117","StudyTime":"120000","AccessionNumber":"","ReferringPhysicianName":"","StudyDescription":"Three slices of LiTS volume 0","StudyInstanceUID":"2.25.3100000","StudyID":"1"}', '{"PatientName":"VOLUME 0^LITS","PatientID":"LITS-0","PatientBirthDate":"","PatientSex":""}', NULL);


--
-- Data for Name: trash; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: trash_files; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: ui_annotations; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: ui_protocols; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: user_identities; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Data for Name: users; Type: TABLE DATA; Schema: public; Owner: -
--



--
-- Name: jobs_id_seq; Type: SEQUENCE SET; Schema: public; Owner: -
--

SELECT pg_catalog.setval('public.jobs_id_seq', 1, false);


--
-- Name: trash_id_seq; Type: SEQUENCE SET; Schema: public; Owner: -
--

SELECT pg_catalog.setval('public.trash_id_seq', 1, false);


--
-- Name: _sqlx_migrations _sqlx_migrations_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public._sqlx_migrations
    ADD CONSTRAINT _sqlx_migrations_pkey PRIMARY KEY (version);


--
-- Name: account_projects account_projects_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.account_projects
    ADD CONSTRAINT account_projects_pkey PRIMARY KEY (account, project);


--
-- Name: auth_refresh_families auth_refresh_families_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.auth_refresh_families
    ADD CONSTRAINT auth_refresh_families_pkey PRIMARY KEY (id);


--
-- Name: auth_refresh_tokens auth_refresh_tokens_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.auth_refresh_tokens
    ADD CONSTRAINT auth_refresh_tokens_pkey PRIMARY KEY (token_hash);


--
-- Name: clients clients_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.clients
    ADD CONSTRAINT clients_pkey PRIMARY KEY (id);


--
-- Name: instances instances_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.instances
    ADD CONSTRAINT instances_pkey PRIMARY KEY (sop_instance_uid);


--
-- Name: instances instances_public_id_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.instances
    ADD CONSTRAINT instances_public_id_key UNIQUE (public_id);


--
-- Name: jobs jobs_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.jobs
    ADD CONSTRAINT jobs_pkey PRIMARY KEY (id);


--
-- Name: limited_accounts limited_accounts_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.limited_accounts
    ADD CONSTRAINT limited_accounts_pkey PRIMARY KEY (account);


--
-- Name: roles roles_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.roles
    ADD CONSTRAINT roles_pkey PRIMARY KEY (id);


--
-- Name: series series_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.series
    ADD CONSTRAINT series_pkey PRIMARY KEY (series_instance_uid);


--
-- Name: series series_public_id_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.series
    ADD CONSTRAINT series_public_id_key UNIQUE (public_id);


--
-- Name: studies studies_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.studies
    ADD CONSTRAINT studies_pkey PRIMARY KEY (study_instance_uid);


--
-- Name: studies studies_public_id_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.studies
    ADD CONSTRAINT studies_public_id_key UNIQUE (public_id);


--
-- Name: trash_files trash_files_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.trash_files
    ADD CONSTRAINT trash_files_pkey PRIMARY KEY (trash_key);


--
-- Name: trash trash_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.trash
    ADD CONSTRAINT trash_pkey PRIMARY KEY (id);


--
-- Name: ui_annotations ui_annotations_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.ui_annotations
    ADD CONSTRAINT ui_annotations_pkey PRIMARY KEY (study_instance_uid);


--
-- Name: ui_protocols ui_protocols_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.ui_protocols
    ADD CONSTRAINT ui_protocols_pkey PRIMARY KEY (id);


--
-- Name: user_identities user_identities_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.user_identities
    ADD CONSTRAINT user_identities_pkey PRIMARY KEY (issuer, subject);


--
-- Name: users users_login_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.users
    ADD CONSTRAINT users_login_key UNIQUE (login);


--
-- Name: users users_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.users
    ADD CONSTRAINT users_pkey PRIMARY KEY (id);


--
-- Name: instances_by_checked; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX instances_by_checked ON public.instances USING btree (checked_ms);


--
-- Name: instances_by_series; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX instances_by_series ON public.instances USING btree (series_instance_uid, instance_number);


--
-- Name: ix_auth_refresh_families_user_id; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX ix_auth_refresh_families_user_id ON public.auth_refresh_families USING btree (user_id);


--
-- Name: ix_auth_refresh_tokens_expires_at; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX ix_auth_refresh_tokens_expires_at ON public.auth_refresh_tokens USING btree (expires_at);


--
-- Name: ix_auth_refresh_tokens_family_id; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX ix_auth_refresh_tokens_family_id ON public.auth_refresh_tokens USING btree (family_id);


--
-- Name: ix_user_identities_user_id; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX ix_user_identities_user_id ON public.user_identities USING btree (user_id);


--
-- Name: jobs_by_state; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX jobs_by_state ON public.jobs USING btree (state);


--
-- Name: series_by_study; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX series_by_study ON public.series USING btree (study_instance_uid);


--
-- Name: studies_by_accession_number; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX studies_by_accession_number ON public.studies USING btree (accession_number);


--
-- Name: studies_by_patient_id; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX studies_by_patient_id ON public.studies USING btree (patient_id);


--
-- Name: studies_by_project; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX studies_by_project ON public.studies USING btree (project);


--
-- Name: studies_by_study_date; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX studies_by_study_date ON public.studies USING btree (study_date);


--
-- Name: trash_by_deleted; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX trash_by_deleted ON public.trash USING btree (deleted_ms);


--
-- Name: trash_files_by_trash; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX trash_files_by_trash ON public.trash_files USING btree (trash_id);


--
-- Name: auth_refresh_tokens auth_refresh_tokens_family_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.auth_refresh_tokens
    ADD CONSTRAINT auth_refresh_tokens_family_id_fkey FOREIGN KEY (family_id) REFERENCES public.auth_refresh_families(id) ON DELETE CASCADE;


--
-- Name: instances instances_series_instance_uid_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.instances
    ADD CONSTRAINT instances_series_instance_uid_fkey FOREIGN KEY (series_instance_uid) REFERENCES public.series(series_instance_uid) ON DELETE CASCADE;


--
-- Name: series series_study_instance_uid_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.series
    ADD CONSTRAINT series_study_instance_uid_fkey FOREIGN KEY (study_instance_uid) REFERENCES public.studies(study_instance_uid) ON DELETE CASCADE;


--
-- Name: trash_files trash_files_trash_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.trash_files
    ADD CONSTRAINT trash_files_trash_id_fkey FOREIGN KEY (trash_id) REFERENCES public.trash(id) ON DELETE CASCADE;


--
-- Name: user_identities user_identities_user_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.user_identities
    ADD CONSTRAINT user_identities_user_id_fkey FOREIGN KEY (user_id) REFERENCES public.users(id) ON DELETE CASCADE;


--
-- PostgreSQL database dump complete
--


