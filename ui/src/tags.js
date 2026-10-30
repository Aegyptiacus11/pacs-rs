// The DICOM tags table of the viewer: names for the common tags, sequences indented.

import { h } from "./util.js";

export const TAG_NAMES = {
  "00020010": "Transfer Syntax UID", "00080005": "Specific Character Set", "00080008": "Image Type",
  "00080016": "SOP Class UID", "00080018": "SOP Instance UID", "00080020": "Study Date", "00080021": "Series Date",
  "00080022": "Acquisition Date", "00080023": "Content Date", "00080030": "Study Time", "00080031": "Series Time",
  "00080032": "Acquisition Time", "00080033": "Content Time", "00080050": "Accession Number", "00080060": "Modality",
  "00080070": "Manufacturer", "00080080": "Institution Name", "00080090": "Referring Physician's Name",
  "00081010": "Station Name", "00081030": "Study Description", "0008103E": "Series Description",
  "00081090": "Manufacturer's Model Name", "00100010": "Patient's Name", "00100020": "Patient ID",
  "00100030": "Patient's Birth Date", "00100040": "Patient's Sex", "00101010": "Patient's Age",
  "00101020": "Patient's Size", "00101030": "Patient's Weight", "00180015": "Body Part Examined",
  "00180050": "Slice Thickness", "00180060": "KVP", "00180088": "Spacing Between Slices",
  "00181020": "Software Versions", "00181030": "Protocol Name", "00181150": "Exposure Time",
  "00181151": "X-Ray Tube Current", "00181152": "Exposure", "00181210": "Convolution Kernel",
  "00185100": "Patient Position", "00180080": "Repetition Time", "00180081": "Echo Time",
  "00180087": "Magnetic Field Strength", "0020000D": "Study Instance UID", "0020000E": "Series Instance UID",
  "00200010": "Study ID", "00200011": "Series Number", "00200012": "Acquisition Number",
  "00200013": "Instance Number", "00200032": "Image Position (Patient)", "00200037": "Image Orientation (Patient)",
  "00200052": "Frame of Reference UID", "00201041": "Slice Location", "00280002": "Samples per Pixel",
  "00280004": "Photometric Interpretation", "00280006": "Planar Configuration", "00280008": "Number of Frames",
  "00280010": "Rows", "00280011": "Columns", "00280030": "Pixel Spacing", "00280100": "Bits Allocated",
  "00280101": "Bits Stored", "00280102": "High Bit", "00280103": "Pixel Representation",
  "00281050": "Window Center", "00281051": "Window Width", "00281052": "Rescale Intercept",
  "00281053": "Rescale Slope", "00281054": "Rescale Type", "7FE00010": "Pixel Data",
};

export function tagValue(element) {
  if (element.Value == null) return element.BulkDataURI ? "(bulk data)" : element.InlineBinary ? "(binary)" : "";
  if (element.vr === "SQ") return `${element.Value.length} item${element.Value.length === 1 ? "" : "s"}`;
  return element.Value.map((v) => (v && typeof v === "object" ? v.Alphabetic ?? JSON.stringify(v) : String(v))).join(" \\ ");
}

export function tagRows(meta, query, depth = 0) {
  const rows = [];
  for (const [t, element] of Object.entries(meta).sort()) {
    const name = TAG_NAMES[t] || "";
    const value = tagValue(element);
    const shown = `(${t.slice(0, 4)},${t.slice(4)})`;
    const matches = !query || `${shown} ${t} ${name} ${value}`.toLowerCase().includes(query);
    const children = element.vr === "SQ" ? element.Value?.flatMap((item) => tagRows(item, query, depth + 1)) ?? [] : [];
    if (matches || children.length) {
      rows.push(
        h(
          "tr",
          {},
          h("td", { class: "t", style: depth ? `padding-left: ${8 + depth * 14}px` : null }, shown),
          h("td", { class: "k" }, name),
          h("td", { class: "vr" }, element.vr || ""),
          h("td", { class: "v" }, value),
        ),
      );
      rows.push(...children);
    }
  }
  return rows;
}

