// The documentation site, https://aegyptiacus11.github.io/pacs-rs/docs/: Starlight over the
// Markdown pages of ../docs, with the sidebar of ../docs/SUMMARY.md.
import { readFileSync } from 'node:fs';
import { defineConfig } from 'astro/config';
import { unified } from '@astrojs/markdown-remark';
import starlight from '@astrojs/starlight';
import starlightGitHubAlerts from 'starlight-github-alerts';
import { pageLinks, withoutTitle } from './src/markdown.mjs';
import { sidebarOf } from './src/pages.mjs';

const summary = readFileSync(new URL('../docs/SUMMARY.md', import.meta.url), 'utf8');

export default defineConfig({
	site: 'https://aegyptiacus11.github.io',
	base: '/pacs-rs/docs',
	outDir: '../target/book',
	// a page keeps the address it has had: /docs/quickstart.html
	build: { format: 'file' },
	// the pages are GitHub's Markdown; the two plugins are in src/markdown.mjs
	markdown: { processor: unified({ remarkPlugins: [withoutTitle, pageLinks] }) },
	integrations: [
		starlight({
			title: 'pacs-rs',
			description: 'An open-source PACS written in Rust, for research and for the clinic.',
			favicon: '/favicon.svg',
			social: [
				{ icon: 'github', label: 'GitHub', href: 'https://github.com/Aegyptiacus11/pacs-rs' },
			],
			editLink: { baseUrl: 'https://github.com/Aegyptiacus11/pacs-rs/edit/main/docs-site/' },
			sidebar: sidebarOf(summary),
			// the pages are outside src/content/docs: callouts and heading links apply there too
			markdown: { processedDirs: ['../docs'] },
			customCss: ['./src/styles/theme.css'],
			plugins: [starlightGitHubAlerts()],
		}),
	],
});
