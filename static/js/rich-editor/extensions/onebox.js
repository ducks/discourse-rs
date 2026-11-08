// extensions/onebox.js: a link the member's typing linkified, once the
// cursor has moved off it, becomes its onebox: a full preview (an onebox
// node, a block) when it is alone on its line at the document's root, else
// its inline title (onebox_inline), as the post cooks. The previews come
// from /onebox and /inline-onebox through Discourse.oneboxes (oneboxes.js,
// the preview's own queue and caches); a link past max_oneboxes_per_post
// stays a link. Both serialize as the url. The link toolbar's Load
// preview forces one (oneboxPluginKey's forceOneboxUrl).
import { PluginKey, Selection } from "prosemirror-state";
import { getLinkify, isWhiteSpace } from "discourse/rich-editor/plugin-utils";
import { toast } from "discourse/rich-editor/toolbar";

export var oneboxPluginKey = new PluginKey("onebox");

// discourse-markdown-it's features/onebox
export var ONEBOX_HREF = /^http|^\/\//i;

export function isTopLevel(href) {
  var split = href.split(/https?:\/\/[^\/]+[\/?]/i);
  var hasExtra = split && split[1] && split[1].length > 0;
  return !hasExtra;
}

function escapeRegExp(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

// "full" if the link at `pos` is alone on its line at the document root
// (renders as a block preview), "inline" otherwise (title-only preview).
export function oneboxTypeAtPos(doc, pos) {
  var $pos = doc.resolve(pos);
  var parent = $pos.parent;
  var index = $pos.index();
  var prev = index > 0 ? parent.child(index - 1) : null;
  var next = index < parent.childCount - 1 ? parent.child(index + 1) : null;
  var isAlone =
    (!prev || prev.type.name === "hard_break") &&
    (!next || next.type.name === "hard_break" || hasTrailingWhitespaceOnly(doc, pos));
  return $pos.depth === 1 && parent.type.name === "paragraph" && isAlone ? "full" : "inline";
}

function isNestedDoc(doc) {
  return doc.type !== doc.type.schema.topNodeType;
}

var ONEBOX_NODE_TYPES = ["onebox", "onebox_inline"];

var oneboxSlotsCache = new WeakMap();

function oneboxSlots(doc) {
  var slots = oneboxSlotsCache.get(doc);
  if (slots) {
    return slots;
  }

  slots = [];
  doc.descendants(function (node, pos) {
    if (takesOneboxSlot(doc, node, pos)) {
      slots.push({ pos: pos, order: cookedOrder(doc, pos) });
    }
  });
  slots.sort(function (a, b) {
    return a.order - b.order;
  });

  oneboxSlotsCache.set(doc, slots);
  return slots;
}

function cookedOrder(doc, pos) {
  var $pos = doc.resolve(pos);
  for (var depth = $pos.depth; depth > 0; depth--) {
    var node = $pos.node(depth);
    if (node.isInline && !node.isLeaf && !node.inlineContent) {
      return doc.content.size + pos;
    }
  }
  return pos;
}

function linkifiedHref(node) {
  var mark = node.marks.find(function (m) {
    return m.type.name === "link" && m.attrs.markup === "linkify";
  });
  return mark ? mark.attrs.href : null;
}

function takesOneboxSlot(doc, node, pos) {
  if (ONEBOX_NODE_TYPES.indexOf(node.type.name) !== -1) {
    return true;
  }

  var href = linkifiedHref(node);

  if (!href || !ONEBOX_HREF.test(href)) {
    return false;
  }

  return oneboxTypeAtPos(doc, pos) === "full" || !isTopLevel(href);
}

function oneboxesPastLimit(doc, maxOneboxes) {
  return new Set(
    oneboxSlots(doc)
      .slice(maxOneboxes)
      .map(function (slot) {
        return slot.pos;
      })
  );
}

function oneboxToLink(state, tr, node, pos, marks) {
  var url = node.attrs.url;
  var link = state.schema.marks.link.create({ href: url, markup: "linkify" });
  var text = state.schema.text(url, link.addToSet(marks));

  tr.replaceWith(
    tr.mapping.map(pos),
    tr.mapping.map(pos + node.nodeSize),
    node.isBlock ? state.schema.nodes.paragraph.create(null, text) : text
  );
}

export function isPastOneboxLimit(doc, pos, maxOneboxes) {
  var order = cookedOrder(doc, pos);
  return (
    oneboxSlots(doc).filter(function (slot) {
      return slot.order < order;
    }).length >= maxOneboxes
  );
}

function maxOneboxesOf(context) {
  var max = parseInt((context.siteSettings || {}).max_oneboxes_per_post, 10);
  return isNaN(max) ? 50 : max;
}

export default {
  nodeSpec: {
    onebox: {
      attrs: { url: {}, html: {} },
      selectable: true,
      group: "block",
      draggable: true,
      parseDOM: [
        {
          tag: "div.onebox-wrapper",
          getAttrs: function (dom) {
            return { url: dom.dataset.oneboxSrc, html: dom.innerHTML };
          },
        },
        {
          tag: "aside.onebox",
          getAttrs: function (dom) {
            return { url: dom.dataset.oneboxSrc, html: dom.outerHTML };
          },
        },
      ],
      toDOM: function (node) {
        var dom = document.createElement("div");
        dom.dataset.oneboxSrc = node.attrs.url;
        dom.classList.add("onebox-wrapper");
        dom.innerHTML = node.attrs.html;
        return dom;
      },
    },
    onebox_inline: {
      attrs: { url: {}, title: {} },
      inline: true,
      group: "inline",
      selectable: true,
      draggable: true,
      parseDOM: [
        {
          tag: "a.inline-onebox",
          getAttrs: function (dom) {
            return { url: dom.getAttribute("href"), title: dom.textContent };
          },
          priority: 60,
        },
      ],
      toDOM: function (node) {
        return ["a", { class: "inline-onebox", href: node.attrs.url, contentEditable: true }, node.attrs.title];
      },
    },
  },
  serializeNode: {
    onebox: function (state, node) {
      state.ensureNewLine();
      state.write(node.attrs.url + "\n\n");
    },
    onebox_inline: function (state, node, parent, index) {
      if (!isWhiteSpace(state.out, state.out.length - 1)) {
        state.write(" ");
      }

      state.text(node.attrs.url, false);

      var nextSibling = parent.childCount > index + 1 ? parent.child(index + 1) : null;
      if (nextSibling && nextSibling.isText && !isWhiteSpace(nextSibling.text, 0)) {
        state.write(" ");
      }
    },
  },

  plugins: function (params) {
    var Plugin = params.pmState.Plugin;
    var Decoration = params.pmView.Decoration;
    var DecorationSet = params.pmView.DecorationSet;
    var getContext = params.getContext;
    var failedUrls = { full: new Set(), inline: new Set() };
    var maxOneboxes = function () {
      return maxOneboxesOf(getContext());
    };

    // Linkified URLs become onebox loading decorations. `forceUrl` skips
    // the selection, top level and failed checks for that URL.
    function scanForOneboxLinks(doc, decoSet, tr, forceUrl) {
      var decorations = [];
      if (isNestedDoc(doc)) {
        return decorations;
      }

      var pastLimit = oneboxesPastLimit(doc, maxOneboxes());
      doc.descendants(function (node, pos) {
        var link = node.marks.find(function (mark) {
          return mark.type.name === "link";
        });

        if (
          link &&
          link.attrs.markup === "linkify" &&
          getLinkify().test(node.text) &&
          !pastLimit.has(pos) &&
          decoSet.find(pos, pos + node.nodeSize).length === 0
        ) {
          var isForced = forceUrl === link.attrs.href;

          if (!isForced && !isOutsideSelection(pos, node.nodeSize, tr)) {
            return;
          }

          var oneboxType = oneboxTypeAtPos(doc, pos);

          // Hold a trailing-whitespace link inline while the cursor is on its
          // line, so the member can keep typing; appendTransaction promotes
          // it.
          if (
            !isForced &&
            oneboxType === "full" &&
            hasTrailingWhitespaceOnly(doc, pos) &&
            selectionInSameBlock(doc, pos, tr.selection)
          ) {
            oneboxType = "inline";
          }

          if (!isForced && isTopLevel(link.attrs.href) && oneboxType === "inline") {
            return;
          }

          if (!isForced && failedUrls[oneboxType].has(link.attrs.href)) {
            return;
          }

          decorations.push(
            Decoration.inline(
              pos,
              pos + node.nodeSize,
              { class: "onebox-loading", nodeName: "span" },
              { oneboxUrl: link.attrs.href, oneboxType: oneboxType, forceRetry: isForced }
            )
          );
        }
      });
      return decorations;
    }

    function removeLoadingDecorations(view, urls, type) {
      var urlSet = typeof urls === "string" ? new Set([urls]) : new Set(urls);
      var decoState = plugin.getState(view.state);
      var toRemove = decoState.find(undefined, undefined, function (spec) {
        return spec.oneboxType === type && urlSet.has(spec.oneboxUrl);
      });
      if (toRemove.length) {
        view.dispatch(view.state.tr.setMeta(plugin, { removeDecorations: toRemove }));
      }
    }

    function showPreviewFailedToast() {
      toast(getContext().i18n("composer.link_toolbar.preview_failed"));
    }

    var plugin = new Plugin({
      key: oneboxPluginKey,
      state: {
        init: function () {
          return DecorationSet.empty;
        },
        apply: function (tr, set) {
          var meta = tr.getMeta(plugin);

          if (meta && meta.removeDecorations) {
            set = set.remove(meta.removeDecorations);
          }

          set = set.map(tr.mapping, tr.doc);

          if (!tr.docChanged) {
            if (meta && meta.inlineOneboxes) {
              var inlineToUpdate = set.find(undefined, undefined, function (spec) {
                return (
                  spec.oneboxType === "inline" &&
                  spec.oneboxUrl &&
                  Object.prototype.hasOwnProperty.call(meta.inlineOneboxes, spec.oneboxUrl)
                );
              });

              var inlineDecorations = inlineToUpdate.map(function (decoration) {
                return Decoration.inline(
                  decoration.from,
                  decoration.to,
                  { class: "onebox-loading", nodeName: "span" },
                  {
                    oneboxUrl: decoration.spec.oneboxUrl,
                    oneboxType: decoration.spec.oneboxType,
                    oneboxTitle: meta.inlineOneboxes[decoration.spec.oneboxUrl],
                    oneboxDataLoaded: true,
                    forceRetry: decoration.spec.forceRetry,
                  }
                );
              });

              set = set.remove(inlineToUpdate).add(tr.doc, inlineDecorations);
            }

            if (meta && meta.oneboxContent) {
              var content = meta.oneboxContent;
              var fullToUpdate = set.find(undefined, undefined, function (spec) {
                return spec.oneboxType === "full" && spec.oneboxUrl === content.url;
              });

              var fullDecorations = fullToUpdate.map(function (decoration) {
                return Decoration.inline(
                  decoration.from,
                  decoration.to,
                  { class: "onebox-loading", nodeName: "span" },
                  {
                    oneboxUrl: decoration.spec.oneboxUrl,
                    oneboxType: decoration.spec.oneboxType,
                    oneboxDataLoaded: true,
                    oneboxHtml: content.html,
                  }
                );
              });

              set = set.remove(fullToUpdate).add(tr.doc, fullDecorations);
            }

            if (!meta || !meta.forceOneboxUrl) {
              return set;
            }
          }

          var decorations = scanForOneboxLinks(tr.doc, set, tr, meta && meta.forceOneboxUrl);

          return set.add(tr.doc, decorations);
        },
      },

      props: {
        decorations: function (state) {
          return plugin.getState(state);
        },
      },

      view: function () {
        var pendingUrls = { inline: new Set(), full: new Set() };

        function processNew(view, allDecorations) {
          var decorations = allDecorations.find(undefined, undefined, function (spec) {
            return !spec.oneboxDataLoaded && !pendingUrls[spec.oneboxType].has(spec.oneboxUrl);
          });

          var inlineUrls = new Set();

          decorations.forEach(function (dec) {
            if (!dec.spec.forceRetry && failedUrls[dec.spec.oneboxType].has(dec.spec.oneboxUrl)) {
              return;
            }

            pendingUrls[dec.spec.oneboxType].add(dec.spec.oneboxUrl);

            if (dec.spec.oneboxType === "inline") {
              inlineUrls.add(dec.spec.oneboxUrl);
            }

            // Full oneboxes, one by one
            if (dec.spec.oneboxType === "full") {
              var oneboxUrl = dec.spec.oneboxUrl;
              var forceRetry = dec.spec.forceRetry;

              if (forceRetry) {
                failedUrls.full.delete(oneboxUrl);
              }

              var failed = function () {
                pendingUrls.full.delete(oneboxUrl);
                failedUrls.full.add(oneboxUrl);
                if (!view.isDestroyed) {
                  removeLoadingDecorations(view, oneboxUrl, "full");
                }
                if (forceRetry) {
                  showPreviewFailedToast();
                }
              };

              processOnebox(oneboxUrl, getContext(), { refresh: forceRetry })
                .then(function (html) {
                  if (!html) {
                    failed();
                    return;
                  }
                  pendingUrls.full.delete(oneboxUrl);
                  if (!view.isDestroyed) {
                    view.dispatch(view.state.tr.setMeta(plugin, { oneboxContent: { url: oneboxUrl, html: html } }));
                  }
                })
                .catch(failed);
            }
          });

          // Inline oneboxes, batched
          if (inlineUrls.size) {
            loadInlineOneboxes(Array.from(inlineUrls), getContext())
              .then(function (inlineOneboxes) {
                inlineUrls.forEach(function (url) {
                  pendingUrls.inline.delete(url);
                });

                if (Object.keys(inlineOneboxes).length > 0 && !view.isDestroyed) {
                  view.dispatch(view.state.tr.setMeta(plugin, { inlineOneboxes: inlineOneboxes }));
                }
              })
              .catch(function () {
                inlineUrls.forEach(function (url) {
                  pendingUrls.inline.delete(url);
                  failedUrls.inline.add(url);
                });
                if (!view.isDestroyed) {
                  removeLoadingDecorations(view, inlineUrls, "inline");
                }
              });
          }
        }

        function processLoaded(view, allDecorations) {
          var decorations = allDecorations.find(undefined, undefined, function (spec) {
            return spec.oneboxDataLoaded;
          });

          var removeDecorations = [];
          var sortedDecos = decorations.sort(function (a, b) {
            return b.from - a.from;
          });
          var tr = view.state.tr;

          sortedDecos.forEach(function (decoration) {
            var nodeAtPos = view.state.doc.nodeAt(decoration.from);

            var matchingLink =
              nodeAtPos &&
              nodeAtPos.marks.find(function (mark) {
                return mark.type.name === "link" && mark.attrs.href === decoration.spec.oneboxUrl;
              });

            if (!nodeAtPos || !nodeAtPos.isText || !matchingLink) {
              return;
            }

            if (decoration.spec.oneboxType === "inline") {
              if (decoration.spec.oneboxTitle) {
                var inlineNode = view.state.schema.nodes.onebox_inline.create({
                  url: nodeAtPos.text,
                  title: decoration.spec.oneboxTitle,
                });

                tr.replaceWith(decoration.from, decoration.to, inlineNode);
              } else {
                failedUrls.inline.add(decoration.spec.oneboxUrl);
                if (decoration.spec.forceRetry) {
                  showPreviewFailedToast();
                }
              }
            } else if (decoration.spec.oneboxType === "full") {
              if (decoration.spec.oneboxHtml) {
                var oneboxNode = view.state.schema.nodes.onebox.create({
                  url: nodeAtPos.text,
                  html: decoration.spec.oneboxHtml,
                });

                var $pos = view.state.doc.resolve(decoration.from);
                if ($pos.parent.type.name === "paragraph") {
                  var from = $pos.before();
                  var to = $pos.after();
                  var cursorOnLine = view.state.selection.from >= from && view.state.selection.to <= to;
                  var nodes = splitParagraphAroundOnebox($pos, oneboxNode);

                  // Somewhere for the cursor to land after the onebox when
                  // it would otherwise end the document, so it isn't left
                  // selecting the block (where typing would replace it).
                  if (nodes[nodes.length - 1] === oneboxNode && to === view.state.doc.content.size) {
                    nodes.push(view.state.schema.nodes.paragraph.create());
                  }

                  tr.replaceWith(from, to, nodes);

                  if (cursorOnLine) {
                    var afterOnebox = from + (nodes[0] === oneboxNode ? 0 : nodes[0].nodeSize) + oneboxNode.nodeSize;
                    tr.setSelection(Selection.near(tr.doc.resolve(afterOnebox), 1));
                  }
                } else {
                  tr.replaceWith(decoration.from, decoration.to, oneboxNode);
                }
              } else {
                failedUrls.full.add(decoration.spec.oneboxUrl);
              }
            }

            removeDecorations.push(decoration);
          });

          if (removeDecorations.length || tr.docChanged) {
            tr.setMeta(plugin, { removeDecorations: removeDecorations });
            view.dispatch(tr.setMeta("addToHistory", false));
          }
        }

        return {
          update: function (view) {
            var decorations = plugin.getState(view.state);
            processNew(view, decorations);
            processLoaded(view, decorations);
          },
        };
      },

      // An inline onebox becomes a full preview once it ends up alone on
      // its line (after Enter, or the text around it removed), as it
      // cooks: back to a link, so scanForOneboxLinks takes it again.
      appendTransaction: function (transactions, prevState, state) {
        var docChanged = transactions.some(function (tr) {
          return tr.docChanged;
        });
        var selectionChanged = !prevState.selection.eq(state.selection);

        if ((!docChanged && !selectionChanged) || isNestedDoc(state.doc)) {
          return;
        }

        // The block the cursor just left, mapped to the near side of any
        // split so it lands in the paragraph left behind (e.g. by Enter).
        var leftPos = prevState.selection.from;
        transactions.forEach(function (t) {
          leftPos = t.mapping.map(leftPos, -1);
        });

        var tr = state.tr;
        var inspected = new Set();
        var pastLimit = oneboxesPastLimit(state.doc, maxOneboxes());

        [state.selection.from, leftPos].forEach(function (position) {
          var range = topBlockRange(state.doc, position);
          if (inspected.has(range.from)) {
            return;
          }
          inspected.add(range.from);

          state.doc.nodesBetween(range.from, range.to, function (node, nodePos) {
            if (node.type.name !== "onebox_inline" || pastLimit.has(nodePos)) {
              return;
            }

            if (oneboxTypeAtPos(state.doc, nodePos) !== "full") {
              return;
            }

            if (selectionInSameBlock(state.doc, nodePos, state.selection)) {
              return;
            }

            oneboxToLink(state, tr, node, nodePos, []);
          });
        });

        pastLimit.forEach(function (pos) {
          var node = state.doc.nodeAt(pos);
          if (!node || ONEBOX_NODE_TYPES.indexOf(node.type.name) === -1) {
            return;
          }

          oneboxToLink(state, tr, node, pos, node.marks);
        });

        return tr.steps.length ? tr.setMeta("addToHistory", false) : null;
      },
    });

    return plugin;
  },
};

// The link at `pos` is followed only by a single whitespace-only text
// node (a trailing space the markdown parser trims, so it still cooks to
// full).
function hasTrailingWhitespaceOnly(doc, pos) {
  var $pos = doc.resolve(pos);
  var parent = $pos.parent;
  var index = $pos.index();

  if (index !== parent.childCount - 2) {
    return false;
  }

  var next = parent.child(index + 1);
  return (
    next.isText &&
    Array.from(next.text).every(function (char) {
      return isWhiteSpace(char, 0);
    })
  );
}

// A full onebox is a block node, so it can't stay inside the paragraph.
// The link can still share its paragraph with other lines through hard
// breaks (shift+enter): those lines go into their own paragraphs around
// the onebox, less the hard breaks (or trailing space) bounding the link.
function splitParagraphAroundOnebox($pos, oneboxNode) {
  var parent = $pos.parent;
  var index = $pos.index();
  var isHardBreak = function (i) {
    return i >= 0 && i < parent.childCount && parent.child(i).type.name === "hard_break";
  };
  var childrenBetween = function (from, to) {
    var children = [];
    for (var i = from; i < to; i++) {
      children.push(parent.child(i));
    }
    return children;
  };

  var nodes = [];

  if (isHardBreak(index - 1)) {
    var before = childrenBetween(0, index - 1);
    if (before.length) {
      nodes.push(parent.type.create(parent.attrs, before));
    }
  }

  nodes.push(oneboxNode);

  if (isHardBreak(index + 1)) {
    var after = childrenBetween(index + 2, parent.childCount);
    if (after.length) {
      nodes.push(parent.type.create(parent.attrs, after));
    }
  }

  return nodes;
}

function topBlockRange(doc, pos) {
  var clamped = Math.max(0, Math.min(pos, doc.content.size));
  var $pos = doc.resolve(clamped);

  if ($pos.depth === 0) {
    var after = $pos.nodeAfter;
    if (after) {
      return { from: clamped, to: clamped + after.nodeSize };
    }
    var before = $pos.nodeBefore;
    if (before) {
      return { from: clamped - before.nodeSize, to: clamped };
    }
    return { from: clamped, to: clamped };
  }

  return { from: $pos.before(1), to: $pos.after(1) };
}

function selectionInSameBlock(doc, pos, selection) {
  var range = topBlockRange(doc, pos);
  return selection.from >= range.from && selection.to <= range.to;
}

function isOutsideSelection(from, to, tr) {
  var selection = tr.selection;
  var doc = tr.doc;

  var nodeEnd = from + to;

  if (selection.from <= nodeEnd && selection.to >= from) {
    return false;
  }

  var text = doc.textBetween(
    selection.to < from ? selection.to : nodeEnd,
    selection.to < from ? from : selection.from,
    " ",
    " "
  );

  for (var i = 0; i < text.length; i++) {
    if (isWhiteSpace(text, i)) {
      return true;
    }
  }

  return false;
}

function oneboxes() {
  return window.Discourse && window.Discourse.oneboxes;
}

// inline-oneboxer's applyInlineOneboxes: { url: title }.
function loadInlineOneboxes(urls, context) {
  if (!oneboxes()) {
    return Promise.resolve({});
  }
  return oneboxes().inlineTitles(urls, { categoryId: context.categoryId, topicId: context.topicId });
}

// oneboxer's load: the preview's HTML, or nothing where /onebox answered
// with the bare link.
function processOnebox(href, context, opts) {
  if (!oneboxes()) {
    return Promise.resolve(null);
  }
  return oneboxes()
    .html(href, { topicId: context.topicId, categoryId: context.categoryId, refresh: !!opts.refresh })
    .then(function (html) {
      if (!html || new RegExp("^<a href=[\"']" + escapeRegExp(href) + "[\"'].*>" + escapeRegExp(href) + "</a>$").test(html)) {
        return null;
      }
      return html;
    });
}
