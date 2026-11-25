// A loader that gives each page the title of its first `# heading`, so that the pages of
// ../docs need no front matter and read the same on GitHub.
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { titleOf } from './pages.mjs';

export function titled(loader) {
	return {
		...loader,
		load: (context) =>
			loader.load({
				...context,
				parseData: async (entry) => {
					if (entry.data.title) return context.parseData(entry);
					const file = fileURLToPath(new URL(entry.filePath, context.config.root));
					const title = titleOf(await readFile(file, 'utf8'), entry.filePath);
					return context.parseData({ ...entry, data: { ...entry.data, title } });
				},
			}),
	};
}
