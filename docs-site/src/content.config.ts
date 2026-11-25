import { defineCollection } from 'astro:content';
import { glob } from 'astro/loaders';
import { docsSchema } from '@astrojs/starlight/schema';
import { titled } from './titled.mjs';

// The pages are the Markdown files of ../docs, where the tests, the README and GitHub's
// readers find them. SUMMARY.md is their table of contents, not a page.
export const collections = {
	docs: defineCollection({
		loader: titled(glob({ base: '../docs', pattern: ['*.md', '!SUMMARY.md'] })),
		schema: docsSchema(),
	}),
};
