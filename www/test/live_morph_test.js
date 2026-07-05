'use strict';

// Tests for the LiveView client's shadow-splice + DOM-morph engine
// (src/live/client.js). Run with: npm run test:js  (node --test + jsdom)

const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const { JSDOM } = require('jsdom');

const CLIENT_SRC = fs.readFileSync(
    path.join(__dirname, '..', '..', 'src', 'live', 'client.js'),
    'utf8'
);

function makeWindow({ runScripts = 'outside-only' } = {}) {
    const dom = new JSDOM(
        '<!doctype html><html><body><div id="root"></div></body></html>',
        { url: 'http://localhost/', runScripts, pretendToBeVisual: true }
    );
    vm.runInContext(CLIENT_SRC, dom.getInternalVMContext());
    return dom.window;
}

function setup(html, options) {
    const window = makeWindow(options);
    const root = window.document.getElementById('root');
    root.innerHTML = html;
    return {
        window,
        document: window.document,
        root,
        morph: window.SoliLiveView.morph,
        splice: window.SoliLiveView.spliceLines
    };
}

// ---------------------------------------------------------------------
// spliceLines
// ---------------------------------------------------------------------

test('spliceLines replaces lines positionally', () => {
    const { splice } = setup('');
    assert.equal(splice('a\nb\nc', 1, 1, ['B']), 'a\nB\nc');
});

test('spliceLines handles pure deletion and insertion', () => {
    const { splice } = setup('');
    assert.equal(splice('a\nb\nc', 1, 1, []), 'a\nc');
    assert.equal(splice('a\nc', 1, 0, ['b']), 'a\nb\nc');
    assert.equal(splice('a\nd', 1, 1, ['b', 'c']), 'a\nb\nc');
});

test('spliceLines distinguishes empty-line replacement from deletion', () => {
    const { splice } = setup('');
    assert.equal(splice('a\nb\nc', 1, 1, ['']), 'a\n\nc');
});

test('spliceLines rejects out-of-bounds and malformed patches', () => {
    const { splice } = setup('');
    assert.equal(splice('a\nb', 1, 5, []), null);
    assert.equal(splice('a\nb', -1, 1, []), null);
    assert.equal(splice('a\nb', 0.5, 1, []), null);
    assert.equal(splice('a\nb', 0, 1, 'not-an-array'), null);
});

// ---------------------------------------------------------------------
// morph: node identity
// ---------------------------------------------------------------------

test('text update preserves sibling element identity', () => {
    const { root, morph } = setup('<div id="keep">hello</div><span>1</span>');
    const keep = root.querySelector('#keep');
    const counter = root.querySelector('span');

    morph(root, '<div id="keep">hello</div><span>2</span>');

    assert.equal(root.querySelector('#keep'), keep);
    assert.equal(root.querySelector('span'), counter);
    assert.equal(counter.textContent, '2');
});

test('multiline whitespace-heavy markup keeps identity across a change', () => {
    const before = '<div class="wrap">\n  <p id="a">one</p>\n  <p id="b">two</p>\n</div>';
    const after = '<div class="wrap">\n  <p id="a">one</p>\n  <p id="b">TWO</p>\n</div>';
    const { root, morph } = setup(before);
    const a = root.querySelector('#a');
    const b = root.querySelector('#b');

    morph(root, after);

    assert.equal(root.querySelector('#a'), a);
    assert.equal(root.querySelector('#b'), b);
    assert.equal(b.textContent, 'TWO');
});

test('keyed reorder preserves node identity', () => {
    const { root, morph } = setup(
        '<ul><li soli-key="a">A</li><li soli-key="b">B</li><li soli-key="c">C</li></ul>'
    );
    const [a, b, c] = root.querySelectorAll('li');

    morph(root, '<ul><li soli-key="c">C</li><li soli-key="a">A</li><li soli-key="b">B</li></ul>');

    const items = root.querySelectorAll('li');
    assert.equal(items[0], c);
    assert.equal(items[1], a);
    assert.equal(items[2], b);
});

test('keyed removal only removes the vanished item', () => {
    const { root, morph } = setup(
        '<ul><li soli-key="a">A</li><li soli-key="b">B</li><li soli-key="c">C</li></ul>'
    );
    const [a, , c] = root.querySelectorAll('li');

    morph(root, '<ul><li soli-key="a">A</li><li soli-key="c">C</li></ul>');

    const items = root.querySelectorAll('li');
    assert.equal(items.length, 2);
    assert.equal(items[0], a);
    assert.equal(items[1], c);
});

test('tag change replaces the node', () => {
    const { root, morph } = setup('<div>x</div>');
    const oldNode = root.firstElementChild;

    morph(root, '<span>x</span>');

    assert.equal(root.firstElementChild.localName, 'span');
    assert.notEqual(root.firstElementChild, oldNode);
});

test('duplicate keys warn but do not crash', () => {
    const { root, morph } = setup(
        '<div soli-key="dup">1</div><div soli-key="dup">2</div>'
    );
    morph(root, '<div soli-key="dup">2</div>');
    assert.equal(root.querySelectorAll('div').length, 1);
});

test('morph populates an empty root (initial render)', () => {
    const { root, morph } = setup('');
    morph(root, '<div id="fresh">hi</div>');
    assert.equal(root.querySelector('#fresh').textContent, 'hi');
});

test('comment nodes update in place', () => {
    const { root, morph } = setup('<!-- v1 --><div>x</div>');
    const div = root.querySelector('div');
    morph(root, '<!-- v2 --><div>x</div>');
    assert.equal(root.firstChild.nodeValue, ' v2 ');
    assert.equal(root.querySelector('div'), div);
});

// ---------------------------------------------------------------------
// morph: attributes
// ---------------------------------------------------------------------

test('attributes are added, updated, and removed', () => {
    const { root, morph } = setup('<div id="d" class="a" data-x="1"></div>');
    const el = root.querySelector('#d');

    morph(root, '<div id="d" class="b" title="t"></div>');

    assert.equal(root.querySelector('#d'), el);
    assert.equal(el.className, 'b');
    assert.equal(el.getAttribute('title'), 't');
    assert.equal(el.hasAttribute('data-x'), false);
});

test('svg subtree morphs without losing namespace', () => {
    const { root, morph } = setup(
        '<svg viewBox="0 0 10 10"><circle cx="1" cy="1" r="1"></circle></svg>'
    );
    const circle = root.querySelector('circle');

    morph(root, '<svg viewBox="0 0 10 10"><circle cx="2" cy="1" r="1"></circle></svg>');

    assert.equal(root.querySelector('circle'), circle);
    assert.equal(circle.getAttribute('cx'), '2');
    assert.equal(circle.namespaceURI, 'http://www.w3.org/2000/svg');
});

// ---------------------------------------------------------------------
// morph: form-state guards
// ---------------------------------------------------------------------

test('user-typed input survives a patch when the server value is unchanged', () => {
    const { root, document, morph } = setup('<input id="f" type="text"><span>1</span>');
    const input = root.querySelector('#f');
    input.focus();
    input.value = 'typed by user';

    morph(root, '<input id="f" type="text"><span>2</span>');

    assert.equal(root.querySelector('#f'), input);
    assert.equal(input.value, 'typed by user');
    assert.equal(document.activeElement, input);
});

test('a focused input is never clobbered, even by a server value change', () => {
    const { root, morph } = setup('<input id="f" type="text" value="server-one">');
    const input = root.querySelector('#f');
    input.focus();
    input.value = 'user text here';
    input.setSelectionRange(9, 9);

    morph(root, '<input id="f" type="text" value="v2">');

    // The attribute follows the server; the live value stays the user's.
    assert.equal(input.getAttribute('value'), 'v2');
    assert.equal(input.value, 'user text here');
    assert.equal(input.selectionStart, 9);
});

test('an unfocused input follows a server value change', () => {
    const { root, morph } = setup('<input id="f" type="text" value="server-one">');
    const input = root.querySelector('#f');
    input.value = 'stale user text';

    morph(root, '<input id="f" type="text" value="v2">');

    assert.equal(input.value, 'v2');
});

test('user-toggled checkbox survives when the server attribute is unchanged', () => {
    const { root, morph } = setup('<input id="c" type="checkbox" checked><span>1</span>');
    const box = root.querySelector('#c');
    box.checked = false;

    morph(root, '<input id="c" type="checkbox" checked><span>2</span>');
    assert.equal(box.checked, false);

    morph(root, '<input id="c" type="checkbox"><span>3</span>');
    assert.equal(box.checked, false);

    morph(root, '<input id="c" type="checkbox" checked><span>4</span>');
    assert.equal(box.checked, true);
});

test('textarea keeps user text unless the server text changes', () => {
    const { root, morph } = setup('<textarea id="t">old</textarea><span>1</span>');
    const area = root.querySelector('#t');
    area.value = 'typed';

    morph(root, '<textarea id="t">old</textarea><span>2</span>');
    assert.equal(area.value, 'typed');

    morph(root, '<textarea id="t">from server</textarea><span>3</span>');
    assert.equal(area.value, 'from server');
    assert.equal(area.defaultValue, 'from server');
});

test('a focused textarea keeps user text across a server text change', () => {
    const { root, morph } = setup('<textarea id="t">old</textarea>');
    const area = root.querySelector('#t');
    area.focus();
    area.value = 'typing in progress';

    morph(root, '<textarea id="t">from server</textarea>');

    assert.equal(area.value, 'typing in progress');
    assert.equal(area.defaultValue, 'from server');
});

test('select follows a server-side selected change after options morph', () => {
    const { root, morph } = setup(
        '<select id="s"><option value="1">1</option><option value="2" selected>2</option></select>'
    );
    const select = root.querySelector('#s');
    assert.equal(select.value, '2');

    morph(
        root,
        '<select id="s"><option value="1" selected>1</option><option value="2">2</option></select>'
    );
    assert.equal(select.value, '1');
});

test('user-picked select option survives when server attributes are unchanged', () => {
    const { root, morph } = setup(
        '<select id="s"><option value="1">1</option><option value="2" selected>2</option></select><span>1</span>'
    );
    const select = root.querySelector('#s');
    select.value = '1';

    morph(
        root,
        '<select id="s"><option value="1">1</option><option value="2" selected>2</option></select><span>2</span>'
    );
    assert.equal(select.value, '1');
});

// ---------------------------------------------------------------------
// morph: soli-ignore and scripts
// ---------------------------------------------------------------------

test('soli-ignore freezes children but keeps own attributes server-driven', () => {
    const { root, document, morph } = setup(
        '<div id="ig" soli-ignore class="a"><span>client widget</span></div>'
    );
    const island = root.querySelector('#ig');
    const clientNode = document.createElement('em');
    clientNode.textContent = 'inserted by client JS';
    island.appendChild(clientNode);

    morph(root, '<div id="ig" soli-ignore class="b"><span>server wants this</span></div>');

    assert.equal(root.querySelector('#ig'), island);
    assert.equal(island.className, 'b');
    assert.equal(island.querySelector('span').textContent, 'client widget');
    assert.equal(island.querySelector('em'), clientNode);
});

test('scripts patched into a live region never execute', () => {
    const { window, root, morph } = setup('<div>before</div>', { runScripts: 'dangerously' });

    morph(root, '<div>before</div><script>window.__scriptRan = true;</script>');

    assert.equal(window.__scriptRan, undefined);
    assert.equal(root.querySelectorAll('script').length, 1);
});

test('an unchanged script node is left untouched on morph', () => {
    const { root, morph } = setup('<script>/* marker */</script><div>1</div>');
    const script = root.querySelector('script');

    morph(root, '<script>/* marker */</script><div>2</div>');

    assert.equal(root.querySelector('script'), script);
});
