// The SEG and RTSTRUCT reader on the real files of tests/data/kidney-seg, without a browser.
// The figures expected were read from the same files with pydicom.
import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { test, expect } from "@playwright/test";
import dicomParser from "dicom-parser";
import { contourCut, labToRgb, OverlayError, readOverlay, segOnImages, topSegments, type RtStruct, type Seg } from "../src/viewer/segmentation";

const DATA = path.resolve(import.meta.dirname, "data/kidney-seg");
const bytes = (name: string) => {
  const b = readFileSync(path.join(DATA, name));
  return b.buffer.slice(b.byteOffset, b.byteOffset + b.byteLength) as ArrayBuffer;
};
const sum = (a: Uint8Array) => a.reduce((n, x) => n + x, 0);

test("a binary SEG: its segments, colors, frames and masks", () => {
  const seg = readOverlay(bytes("seg.dcm")) as Seg;
  expect(seg.kind).toBe("SEG");
  expect(seg.segments.map((s) => [s.number, s.label])).toEqual([
    [1, "Kidney"],
    [2, "Mass"],
    [3, "Cyst"],
  ]);
  expect(seg.frames).toHaveLength(101);
  expect(seg.frames.filter((f) => f.segment === 2)).toHaveLength(14);
  expect(seg.referencedSeries).toEqual(["1.3.6.1.4.1.14519.5.2.1.6450.2626.275857414129959716213130533744"]);
  // bits unpacked across frame boundaries as pydicom reads them
  expect(sum(seg.mask(0))).toBe(44);
  expect(seg.frames[50].segment).toBe(2);
  expect(sum(seg.mask(50))).toBe(178);
  expect(seg.frames.reduce((n, _, i) => n + sum(seg.mask(i)), 0)).toBe(349872);
});

test("a fractional SEG: a pixel is in from half the Maximum Fractional Value up", () => {
  // seg-fractional.dcm: frames 51 and 1 of seg.dcm as 8-bit probabilities out of 255, the
  // first one's mask at 200, the second one's at 100
  const seg = readOverlay(bytes("seg-fractional.dcm")) as Seg;
  expect(seg.frames.map((f) => f.segment)).toEqual([2, 1]);
  expect(sum(seg.mask(0))).toBe(178);
  expect(sum(seg.mask(1))).toBe(0);
});

test("a SEG on the images it names: 24 frames on the 8 slices kept, the rest counted as missed", () => {
  const seg = readOverlay(bytes("seg.dcm")) as Seg;
  const sops = readdirSync(path.join(DATA, "ct")).map(sopOf);
  const { layers, missed } = segOnImages(seg, sops.map((sop) => ({ sop, frame: 1, plane: null })));
  expect(missed).toBe(101 - 24);
  expect(layers.every((m) => m !== null)).toBe(true);
  // every segment is somewhere, each image's in rising number
  expect([...new Set(layers.flatMap((m) => m!.map((l) => l.segment)))].sort()).toEqual([1, 2, 3]);
  for (const m of layers) expect(m!.map((l) => l.segment)).toEqual([...m!.map((l) => l.segment)].sort());
  // as one labelmap, the higher numbers lie over the lower
  const size = seg.rows * seg.columns;
  const values = new Set(layers.flatMap((m) => [...new Set(topSegments(m!, size))]));
  expect([...values].sort()).toEqual([0, 1, 2, 3]);
});

test("overlapping segments each keep their mask on an image; as one labelmap the higher wins", () => {
  // seg-overlap.dcm: each Kidney frame of seg.dcm also holds the Mass of its image
  const seg = readOverlay(bytes("seg-overlap.dcm")) as Seg;
  const sops = readdirSync(path.join(DATA, "ct")).map(sopOf);
  const { layers } = segOnImages(seg, sops.map((sop) => ({ sop, frame: 1, plane: null })));
  const size = seg.rows * seg.columns;
  let overlap = 0;
  for (const image of layers) {
    const kidney = image!.find((l) => l.segment === 1)!.mask, mass = image!.find((l) => l.segment === 2)!.mask;
    const top = topSegments(image!, size);
    for (let p = 0; p < size; p++)
      if (mass[p]) {
        // the kidney is still there under its mass, and a labelmap shows the mass
        expect(kidney[p]).toBe(1);
        expect(top[p]).toBe(2);
        overlap++;
      }
  }
  expect(overlap).toBe(6653);
});

test("a SEG whose rows run the other way lands on the anatomy, not mirrored", () => {
  // This SEG's frames go bottom to top (Image Orientation 1\\0\\0\\0\\-1\\0) while the CT's go top to
  // bottom: laid pixel for pixel, its kidney fell on fat (-10 HU on average); carried through
  // its own geometry, on enhancing kidney (135 HU, as pydicom finds with the mask flipped).
  const seg = readOverlay(bytes("seg.dcm")) as Seg;
  expect(seg.frames[0].orientation).toEqual([1, 0, 0, 0, -1, 0]);
  const ct = dicomParser.parseDicom(new Uint8Array(bytes("ct/069.dcm")));
  const n = (t: string) => ct.string(t)!.split("\\").map(Number);
  const [rows, columns] = [ct.uint16("x00280010")!, ct.uint16("x00280011")!];
  const iop = n("x00200037");
  const plane = {
    origin: n("x00200032"),
    row: iop.slice(0, 3),
    column: iop.slice(3),
    spacing: n("x00280030") as [number, number],
    normal: [0, 0, 1],
    rows,
    columns,
  };
  const pixel = ct.elements.x7fe00010;
  const raw = new Int16Array(ct.byteArray.buffer.slice(ct.byteArray.byteOffset + pixel.dataOffset, ct.byteArray.byteOffset + pixel.dataOffset + pixel.length));
  const [slope, intercept] = [Number(ct.string("x00281053")), Number(ct.string("x00281052"))];
  const meanHu = (map: Uint16Array, segment: number) => {
    let total = 0, count = 0;
    map.forEach((v, p) => v === segment && ((total += raw[p] * slope + intercept), count++));
    return Math.round(total / count);
  };
  const sop = ct.string("x00080018")!;
  const top = (p: typeof plane | null, size: number) => topSegments(segOnImages(seg, [{ sop, frame: 1, plane: p }]).layers[0]!, size);
  const placed = top(plane, rows * columns);
  const asStored = top(null, rows * columns);
  expect(meanHu(asStored, 1)).toBe(-10);
  expect(meanHu(placed, 1)).toBe(135);

  // on images twice as fine, the same anatomy covers four times the pixels, with no holes:
  // each SEG pixel placed on one image pixel left three of four empty
  const fine = { ...plane, rows: rows * 2, columns: columns * 2, spacing: plane.spacing.map((x) => x / 2) as [number, number] };
  const finer = top(fine, fine.rows * fine.columns);
  const count = (m: Uint16Array) => m.reduce((n, v) => n + (v > 0 ? 1 : 0), 0);
  expect(count(finer)).toBeGreaterThan(3.8 * count(placed));
  expect(count(finer)).toBeLessThan(4.2 * count(placed));
  // a kidney pixel's four neighbours on the fine grid are kidney too
  const at = finer.findIndex((v, p) => v === 1 && finer[p + 1] === 1 && finer[p + fine.columns] === 1);
  expect(at).toBeGreaterThan(-1);
});

test("an RTSTRUCT: its region, color and contours", () => {
  const rt = readOverlay(bytes("rtstruct.dcm")) as RtStruct;
  expect(rt.kind).toBe("RTSTRUCT");
  expect(rt.rois).toHaveLength(1);
  expect(rt.rois[0].name).toBe("RT KIDNEY - 1");
  expect(rt.rois[0].color).toEqual([255, 255, 0]);
  expect(rt.rois[0].contours).toHaveLength(14);
  expect(rt.rois[0].contours[0].points).toHaveLength(22);
  expect(rt.rois[0].contours[0].points[0]).toEqual([-107.11739552979, 65.017254975302, -58.5]);
  expect(rt.rois[0].contours.every((c) => c.closed && c.sop)).toBe(true);
});

test("a contour is whole on its slice, and two crossings on a plane across it", () => {
  const contour = (readOverlay(bytes("rtstruct.dcm")) as RtStruct).rois[0].contours[0];
  const center = [0, 1, 2].map((k) => contour.points.reduce((n, p) => n + p[k], 0) / contour.points.length);
  // the axial plane it lies on
  expect(contourCut(contour, center, [0, 0, 1], 2.5).polygon).toHaveLength(22);
  // a sagittal plane through its middle cuts it once, in one segment
  const cut = contourCut(contour, center, [1, 0, 0], 2.5);
  expect(cut.polygon).toBeNull();
  expect(cut.segments).toHaveLength(1);
  expect(cut.segments[0].every((p) => Math.abs(p[0] - center[0]) < 1e-6)).toBe(true);
  // an axial plane a slice away shows nothing
  expect(contourCut(contour, [center[0], center[1], center[2] + 2.5], [0, 0, 1], 2.5)).toEqual({ polygon: null, segments: [] });
});

test("files that are not a SEG or RTSTRUCT, or not readable, are refused with why", () => {
  expect(() => readOverlay(bytes("ct/065.dcm"))).toThrow(new OverlayError("a CT, neither a SEG nor an RTSTRUCT"));
  // the SEG said to be RLE compressed: the same file, its transfer syntax changed
  const seg = new Uint8Array(bytes("seg.dcm"));
  const explicit = new TextEncoder().encode("1.2.840.10008.1.2.1\0");
  const at = seg.findIndex((_, i) => explicit.every((b, j) => seg[i + j] === b));
  expect(at).toBeGreaterThan(0);
  seg.set(new TextEncoder().encode("1.2.840.10008.1.2.5\0"), at);
  expect(() => readOverlay(seg.buffer)).toThrow(/stored compressed \(1\.2\.840\.10008\.1\.2\.5\)/);
  // cut short
  expect(() => readOverlay(bytes("seg.dcm").slice(0, 200_000))).toThrow(OverlayError);
  expect(() => readOverlay(new ArrayBuffer(10))).toThrow(OverlayError);
});

test("DICOM CIELab becomes the sRGB it stands for", () => {
  // white and the Kidney's color
  expect(labToRgb([65535, 32896, 32896])).toEqual([255, 255, 255]);
  const [r, g, b] = labToRgb([34296, 40957, 39424]);
  expect(r).toBeGreaterThan(g);
  expect(r).toBeGreaterThan(b);
});

/** A CT file's SOP Instance UID. */
function sopOf(file: string) {
  return dicomParser.parseDicom(new Uint8Array(bytes(`ct/${file}`))).string("x00080018")!;
}
