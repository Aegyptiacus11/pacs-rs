import assert from 'node:assert/strict';
import { test } from 'node:test';
import { addressOf, sidebarOf, titleOf } from '../src/pages.mjs';
import { pageLinks, withoutTitle } from '../src/markdown.mjs';

test('the title of a page is its first heading, outside code', () => {
	assert.equal(titleOf('intro\n\n```\n# a comment\n```\n# The `real` title #\n# Second', 'a.md'), 'The real title');
	assert.throws(() => titleOf('## only a section\n', 'docs/a.md'), /docs\/a\.md: no "# title" line/);
	assert.throws(() => titleOf('```\n# in code\n```\n', 'a.md'), /no "# title" line/);
});

test('the sidebar is what SUMMARY.md lists, in its order', () => {
	const summary = '# Summary\n\n[About](introduction.md)\n\n# Start\n\n- [Quickstart](quickstart.md)\n- [Two ways](two-ways.md)\n';
	assert.deepEqual(sidebarOf(summary), [
		{ label: 'About', slug: 'introduction' },
		{ label: 'Start', items: [{ label: 'Quickstart', slug: 'quickstart' }, { label: 'Two ways', slug: 'two-ways' }] },
	]);
});

test('a SUMMARY.md that cannot be a sidebar is refused, with the line', () => {
	assert.throws(() => sidebarOf('# Start\n\n- Quickstart\n'), /line 3: neither/);
	assert.throws(() => sidebarOf('- [Quickstart](quickstart.md)\n'), /line 1: a list item before any/);
	assert.throws(() => sidebarOf('# Start\n\n- [Out](../README.md)\n'), /line 3: neither/);
	assert.throws(() => sidebarOf('# Start\n\n# Next\n- [A](a.md)\n'), /the group "Start" has no page/);
	assert.throws(() => sidebarOf('# Summary\n'), /no page/);
});

test('a link to a page of docs leads to that page of the site, and no other link changes', () => {
	assert.equal(addressOf('login.md'), 'login.html');
	assert.equal(addressOf('login.md#roles'), 'login.html#roles');
	for (const kept of ['https://example.org/a.md', '#here', 'images/a.jpg', '../README.md', 'sample/00001.dcm']) {
		assert.equal(addressOf(kept), kept);
	}
});

test('a rendered page loses its title line and has its links rewritten', () => {
	const tree = {
		type: 'root',
		children: [
			{ type: 'heading', depth: 1, children: [] },
			{ type: 'paragraph', children: [{ type: 'link', url: 'audit.md#format', children: [] }] },
			{ type: 'heading', depth: 1, children: [] },
			{ type: 'definition', url: 'login.md' },
		],
	};
	withoutTitle()(tree);
	pageLinks()(tree);
	assert.deepEqual(tree.children.map((node) => node.type), ['paragraph', 'heading', 'definition']);
	assert.equal(tree.children[0].children[0].url, 'audit.html#format');
	assert.equal(tree.children[2].url, 'login.html');
	const untitled = { type: 'root', children: [{ type: 'paragraph', children: [] }] };
	withoutTitle()(untitled);
	assert.equal(untitled.children.length, 1);
});
