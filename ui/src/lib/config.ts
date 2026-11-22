// What the server says about itself, and where it is.

export const config: { dicomweb: string; version: string; auth?: boolean; provider?: boolean; trash_days?: number } = await fetch("config.json").then((r) => r.json());
/** The REST API: the server's root, next to this page's folder. */
export const ROOT = new URL("../", location.href);
/** DICOMweb, wherever the server serves it. */
export const WEB = new URL(config.dicomweb.replace(/^\/+|\/+$/g, "") + "/", ROOT);

/** What happens to what is deleted: a while in the trash, or gone at once. */
export const afterDelete =
  config.trash_days && config.trash_days > 0
    ? `will be moved to the trash, where an administrator can restore ${config.trash_days === 1 ? "it for a day" : `it for ${config.trash_days} days`}.`
    : "will be removed from storage. This can't be undone.";
