import { PluginKey } from 'prosemirror-state';
export const DEFAULT_ID = 'codemark';
export const MAX_MATCH = 100;
export const pluginKey = new PluginKey(DEFAULT_ID);
export function getMarkType(view, opts) {
    var _a, _b;
    if ('schema' in view)
        return (_a = opts === null || opts === void 0 ? void 0 : opts.markType) !== null && _a !== void 0 ? _a : view.schema.marks.code;
    return (_b = opts === null || opts === void 0 ? void 0 : opts.markType) !== null && _b !== void 0 ? _b : view.state.schema.marks.code;
}
export function safeResolve(doc, pos) {
    return doc.resolve(Math.min(Math.max(1, pos), doc.nodeSize - 2));
}
//# sourceMappingURL=utils.js.map