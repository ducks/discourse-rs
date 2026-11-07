import { Plugin } from 'prosemirror-state';
import { Decoration, DecorationSet } from 'prosemirror-view';
import { getMarkType, pluginKey, safeResolve } from './utils';
import { createInputRule } from './inputRules';
import { onArrowLeft, onArrowRight, onBackspace, onBacktick, onDelete, stepOutside, stepOutsideNextTrAndPass, } from './actions';
function toDom() {
    const span = document.createElement('span');
    span.classList.add('fake-cursor');
    return span;
}
export function getDecorationPlugin(opts) {
    const plugin = new Plugin({
        key: pluginKey,
        appendTransaction: (trs, oldState, newState) => {
            var _a;
            const prev = plugin.getState(oldState);
            const meta = (_a = trs[0]) === null || _a === void 0 ? void 0 : _a.getMeta(plugin);
            if ((prev === null || prev === void 0 ? void 0 : prev.next) || (meta === null || meta === void 0 ? void 0 : meta.action) === 'click') {
                return stepOutside(newState, getMarkType(newState, opts));
            }
            return null;
        },
        state: {
            init: () => null,
            apply(tr, value, oldState, state) {
                var _a;
                const meta = tr.getMeta(plugin);
                if ((meta === null || meta === void 0 ? void 0 : meta.action) === 'next')
                    return { next: true };
                const markType = getMarkType(state, opts);
                const nextMark = markType.isInSet((_a = state.storedMarks) !== null && _a !== void 0 ? _a : state.doc.resolve(tr.selection.from).marks());
                const inCode = markType.isInSet(state.doc.resolve(tr.selection.from).marks());
                const nextCode = markType.isInSet(safeResolve(state.doc, tr.selection.from + 1).marks());
                const startOfLine = tr.selection.$from.parentOffset === 0;
                if (!tr.selection.empty)
                    return null;
                if (!nextMark && nextCode && (!inCode || startOfLine)) {
                    // |`code`
                    return { active: true, side: -1 };
                }
                if (nextMark && (!inCode || startOfLine)) {
                    // `|code`
                    return { active: true, side: 0 };
                }
                if (!nextMark && inCode && !nextCode) {
                    // `code`|
                    return { active: true, side: 0 };
                }
                if (nextMark && inCode && !nextCode) {
                    // `code|`
                    return { active: true, side: -1 };
                }
                return null;
            },
        },
        props: {
            attributes: (state) => {
                var _a;
                const { active = false } = (_a = plugin.getState(state)) !== null && _a !== void 0 ? _a : {};
                return Object.assign({}, (active ? { class: 'no-cursor' } : {}));
            },
            decorations: (state) => {
                var _a;
                const { active, side } = (_a = plugin.getState(state)) !== null && _a !== void 0 ? _a : {};
                if (!active)
                    return DecorationSet.empty;
                const deco = Decoration.widget(state.selection.from, toDom, { side });
                return DecorationSet.create(state.doc, [deco]);
            },
            handleKeyDown(view, event) {
                switch (event.key) {
                    case '`':
                        return onBacktick(view, plugin, event, getMarkType(view, opts));
                    case 'ArrowRight':
                        return onArrowRight(view, plugin, event, getMarkType(view, opts));
                    case 'ArrowLeft':
                        return onArrowLeft(view, plugin, event, getMarkType(view, opts));
                    case 'Backspace':
                        return onBackspace(view, plugin, event, getMarkType(view, opts));
                    case 'Delete':
                        return onDelete(view, plugin, event, getMarkType(view, opts));
                    case 'ArrowUp':
                    case 'ArrowDown':
                    case 'Home':
                    case 'End':
                        return stepOutsideNextTrAndPass(view, plugin);
                    case 'e':
                    case 'a':
                        if (!event.ctrlKey)
                            return false;
                        return stepOutsideNextTrAndPass(view, plugin);
                    default:
                        return false;
                }
            },
            handleClick(view) {
                return stepOutsideNextTrAndPass(view, plugin, 'click');
            },
        },
    });
    return plugin;
}
export function codemark(opts) {
    const cursorPlugin = getDecorationPlugin(opts);
    const inputRule = createInputRule(cursorPlugin, opts);
    const rules = [cursorPlugin, inputRule];
    return rules;
}
//# sourceMappingURL=plugin.js.map