// What ties the pages of ../docs to this site, kept free of Astro so that it can be tested:
// the title of a page, the sidebar from SUMMARY.md, and the address a link between pages
// becomes.

/** The title of a page: the text of its first `# heading`, outside code blocks. */
export function titleOf(markdown, name) {
	let fenced = false;
	for (const line of markdown.split('\n')) {
		if (/^\s*(```|~~~)/.test(line)) fenced = !fenced;
		const heading = !fenced && /^#\s+(.+?)\s*#*\s*$/.exec(line);
		if (heading) return heading[1].replace(/`/g, '');
	}
	throw new Error(`${name}: no "# title" line, so the page has no title`);
}

/**
 * The sidebar SUMMARY.md describes: a `# heading` opens a group, `- [label](page.md)` is a
 * page of the group, and `[label](page.md)` before any group is a link of its own.
 */
export function sidebarOf(summary) {
	const sidebar = [];
	let group = null;
	for (const [index, line] of summary.split('\n').entries()) {
		const where = `SUMMARY.md, line ${index + 1}`;
		if (!line.trim() || line.trim() === '# Summary') continue;
		const heading = /^#\s+(.+)$/.exec(line);
		if (heading) {
			group = { label: heading[1].trim(), items: [] };
			sidebar.push(group);
			continue;
		}
		const page = /^(- )?\[([^\]]+)\]\(([a-z0-9-]+)\.md\)\s*$/.exec(line);
		if (!page) throw new Error(`${where}: neither a "# group" nor a "[label](page.md)": ${line}`);
		const item = { label: page[2], slug: page[3] };
		if (page[1] && !group) throw new Error(`${where}: a list item before any "# group"`);
		(page[1] ? group.items : sidebar).push(item);
	}
	for (const entry of sidebar) {
		if (entry.items && entry.items.length === 0) {
			throw new Error(`SUMMARY.md: the group "${entry.label}" has no page`);
		}
	}
	if (sidebar.length === 0) throw new Error('SUMMARY.md: no page');
	return sidebar;
}

/**
 * Where a link written between two pages of ../docs leads on the site: `login.md#roles`
 * becomes `login.html#roles`, beside the page that links to it. Any other link is returned
 * as it is.
 */
export function addressOf(url) {
	const page = /^([a-z0-9-]+)\.md(#.*)?$/.exec(url);
	return page ? `${page[1]}.html${page[2] ?? ''}` : url;
}
