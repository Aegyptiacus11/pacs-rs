// What the server says about itself, and where it is.

export const config: { dicomweb: string; version: string; auth?: boolean } = await fetch("config.json").then((r) => r.json());
/** The REST API: the server's root, next to this page's folder. */
export const ROOT = new URL("../", location.href);
/** DICOMweb, wherever the server serves it. */
export const WEB = new URL(config.dicomweb.replace(/^\/+|\/+$/g, "") + "/", ROOT);
