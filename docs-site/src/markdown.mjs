// The two changes made to a page of ../docs as it is rendered. The pages stay written for
// GitHub too: a `# title` line, and links to each other by file name.
import { addressOf } from './pages.mjs';

function walk(node, visit) {
	visit(node);
	for (const child of node.children ?? []) walk(child, visit);
}

/** The first `# heading` is left out: Starlight writes the title itself. */
export function withoutTitle() {
	return (tree) => {
		const index = tree.children.findIndex((node) => node.type === 'heading' && node.depth === 1);
		if (index >= 0) tree.children.splice(index, 1);
	};
}

/** Links to `page.md` lead to that page of the site. */
export function pageLinks() {
	return (tree) => {
		walk(tree, (node) => {
			if (node.type === 'link' || node.type === 'definition') node.url = addressOf(node.url);
		});
	};
}
