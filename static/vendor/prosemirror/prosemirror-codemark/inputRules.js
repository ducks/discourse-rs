import { Plugin, TextSelection } from 'prosemirror-state';
import { getMarkType, MAX_MATCH } from './utils';
function stopMatch(markType, view, from, to) {
    var _a;
    const stored = markType.isInSet((_a = view.state.storedMarks) !== null && _a !== void 0 ? _a : view.state.doc.resolve(from).marks());
    const range = view.state.doc.rangeHasMark(from, to, markType);
    // Don't create it if there is code in between!
    if (stored || range)
        return true;
    return false;
}
const markBefore = {
    match: /`((?:[^`\w]|[\w])+)`$/,
    handler: (markType, view, text, match, from, to, plugins) => {
        if (stopMatch(markType, view, from, to))
            return false;
        const code = match[1];
        const mark = markType.create();
        const pos = from + code.length;
        const tr = view.state.tr.delete(from, to).insertText(code).addMark(from, pos, mark);
        const selected = tr.setSelection(TextSelection.create(tr.doc, pos)).removeStoredMark(markType);
        const withMeta = selected.setMeta(plugins.input, {
            transform: selected,
            from,
            to,
            text: `\`${code}${text}`,
        });
        view.dispatch(withMeta);
        return true;
    },
};
const markAfter = {
    match: /^`((?:[^`\w]|[\w])+)`/,
    handler: (markType, view, text, match, from, to, plugins) => {
        if (stopMatch(markType, view, from, to))
            return false;
        const mark = markType.create();
        const code = match[1];
        const pos = from;
        const tr = view.state.tr
            .delete(from, to)
            .insertText(code)
            .addMark(from, from + code.length, mark);
        const selected = tr
            .setSelection(TextSelection.create(tr.doc, pos))
            .addStoredMark(markType.create());
        const withMeta = selected.setMeta(plugins.input, {
            transform: selected,
            from,
            to,
            text: `\`${code}${text}`,
        });
        view.dispatch(withMeta);
        return true;
    },
};
function run(markType, view, from, to, text, plugins) {
    if (view.composing)
        return false;
    const { state } = view;
    const $from = state.doc.resolve(from);
    if ($from.parent.type.spec.code)
        return false;
    const leafText = '\ufffc';
    const textBefore = $from.parent.textBetween(Math.max(0, $from.parentOffset - MAX_MATCH), $from.parentOffset, undefined, leafText) + text;
    const textAfter = text +
        $from.parent.textBetween($from.parentOffset, Math.min($from.parent.nodeSize - 2, $from.parentOffset + MAX_MATCH), undefined, leafText);
    const matchB = markBefore.match.exec(textBefore);
    const matchA = markAfter.match.exec(textAfter);
    if (matchB) {
        const handled = markBefore.handler(markType, view, text, matchB, from - matchB[0].length + text.length, to, plugins);
        if (handled)
            return handled;
    }
    if (matchA)
        return markAfter.handler(markType, view, text, matchA, from, to + matchA[0].length - text.length, plugins);
    return false;
}
export function createInputRule(cursorPlugin, opts) {
    const plugin = new Plugin({
        isInputRules: true,
        state: {
            init: () => null,
            apply(tr, prev) {
                const meta = tr.getMeta(plugin);
                if (meta)
                    return meta;
                return tr.selectionSet || tr.docChanged ? null : prev;
            },
        },
        props: {
            handleTextInput(view, from, to, text) {
                const markType = getMarkType(view, opts);
                return run(markType, view, from, to, text, { input: plugin, cursor: cursorPlugin });
            },
        },
    });
    return plugin;
}
//# sourceMappingURL=inputRules.js.map