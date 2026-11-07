// checklist's lib/rich-editor-extension.js: [ ] and [x] as check nodes
// (check_open in the node tree) that a click toggles, a bullet list item
// starting with one as a checklist item (Enter continues the checklist, an
// empty item ends it, Backspace and ArrowLeft step over the box), and the
// toggleBulletList, toggleOrderedList and toggleChecklist commands the
// toolbar's list menu runs.
var CHECKED_REGEX = /\bchecked\b/;

function hasCheckedClass(className) {
  return CHECKED_REGEX.test(className);
}

export default {
  nodeSpec: {
    check: {
      attrs: { checked: { default: false } },
      inline: true,
      group: "inline",
      draggable: true,
      selectable: false,
      toDOM: function (node) {
        return [
          "span",
          {
            class: node.attrs.checked ? "chcklst-box checked fa fa-square-check-o" : "chcklst-box fa fa-square-o",
          },
        ];
      },
      parseDOM: [
        {
          tag: "span.chcklst-box",
          getAttrs: function (dom) {
            return { checked: hasCheckedClass(dom.className) };
          },
        },
      ],
    },
  },

  inputRules: [
    {
      match: /(^|\s)\[(x? ?)]$/,
      handler: function (state, match, start, end) {
        var checkNode = state.schema.nodes.check.create({ checked: match[2] === "x" });
        var spaceNode = state.schema.text(" ");
        return state.tr.replaceWith(start + match[1].length, end, [checkNode, spaceNode]);
      },
    },
  ],

  parse: {
    check_open: {
      node: "check",
      getAttrs: function (token) {
        return { checked: hasCheckedClass(token.attrGet("class")) };
      },
    },
    check_close: { noCloseToken: true, ignore: true },
  },

  serializeNode: {
    check: function (state, node) {
      state.write(node.attrs.checked ? "[x]" : "[ ]");
    },
  },

  plugins: function (params) {
    var Plugin = params.pmState.Plugin;
    var Decoration = params.pmView.Decoration;
    var DecorationSet = params.pmView.DecorationSet;
    var schema = params.schema;
    var changedDescendants = params.utils.changedDescendants;
    var checkType = schema.nodes.check;
    var listItemType = schema.nodes.list_item;
    var bulletListType = schema.nodes.bullet_list;

    function startsWithCheck(node) {
      return !!(node && node.isTextblock && node.firstChild && node.firstChild.type === checkType);
    }

    function hasSpaceAfterCheck(node) {
      var secondChild = node.childCount > 1 ? node.child(1) : null;
      return !!(secondChild && secondChild.isText && secondChild.text && secondChild.text[0] === " ");
    }

    function findBulletListContext($from) {
      for (var depth = $from.depth; depth > 0; depth--) {
        if ($from.node(depth).type === listItemType) {
          var list = $from.node(depth - 1);
          if (list && list.type === bulletListType) {
            return { bulletListDepth: depth - 1, listItemDepth: depth };
          }
          return null;
        }
      }
      return null;
    }

    function inBulletListItem(doc, pos) {
      return findBulletListContext(doc.resolve(pos)) !== null;
    }

    function ensureSpaceAfterChecks(tr, oldState, newState) {
      var positionsToInsert = [];

      changedDescendants(oldState.doc, newState.doc, function (node, pos) {
        if (!startsWithCheck(node) || !inBulletListItem(newState.doc, pos)) {
          return;
        }

        if (!hasSpaceAfterCheck(node)) {
          positionsToInsert.push(pos + 1 + node.firstChild.nodeSize);
        }
      });

      for (var i = positionsToInsert.length - 1; i >= 0; i--) {
        tr.insert(positionsToInsert[i], schema.text(" "));
      }
    }

    function exitChecklist(tr, ctx) {
      var bulletListDepth = ctx.bulletListDepth;
      var listItemDepth = ctx.listItemDepth;
      var selection = tr.selection;
      var $from = selection.$from;
      var bulletList = $from.node(bulletListDepth);
      var listItemIndex = $from.index(bulletListDepth);
      var prevListItem = bulletList.child(listItemIndex - 1);
      var bulletListStart = $from.before(bulletListDepth);

      tr.delete($from.before(listItemDepth), $from.after(listItemDepth));

      var prevOffset = 1;
      for (var i = 0; i < listItemIndex - 1; i++) {
        prevOffset += bulletList.child(i).nodeSize;
      }
      var prevItemStart = tr.mapping.map(bulletListStart + prevOffset);
      tr.delete(prevItemStart, prevItemStart + prevListItem.nodeSize);

      var mappedListStart = tr.mapping.map(bulletListStart);
      var listAfter = tr.doc.nodeAt(mappedListStart);

      if (listAfter && listAfter.childCount > 0) {
        var listEnd = mappedListStart + listAfter.nodeSize;
        tr.insert(listEnd, schema.nodes.paragraph.create());
        tr.setSelection(selection.constructor.near(tr.doc.resolve(listEnd + 1)));
      } else {
        tr.replaceWith(
          mappedListStart,
          mappedListStart + ((listAfter && listAfter.nodeSize) || 0),
          schema.nodes.paragraph.create()
        );
        tr.setSelection(selection.constructor.near(tr.doc.resolve(mappedListStart + 1)));
      }
      return tr;
    }

    function handleChecklistContinuation(tr, transactions) {
      if (
        !transactions.some(function (t) {
          return t.docChanged;
        })
      ) {
        return null;
      }

      var selection = tr.selection;
      var $from = selection.$from;

      if (!selection.empty) {
        return null;
      }

      var parent = $from.parent;
      if (!parent.isTextblock || parent.content.size !== 0) {
        return null;
      }

      var ctx = findBulletListContext($from);
      if (!ctx) {
        return null;
      }

      var bulletList = $from.node(ctx.bulletListDepth);
      var listItemIndex = $from.index(ctx.bulletListDepth);
      if (listItemIndex === 0) {
        return null;
      }

      var prevParagraph = bulletList.child(listItemIndex - 1).firstChild;
      if (!startsWithCheck(prevParagraph)) {
        return null;
      }

      if (prevParagraph.content.size > 2) {
        var checkNode = checkType.create({ checked: false });
        tr.insert($from.pos, [checkNode, schema.text(" ")]);
        tr.setSelection(selection.constructor.near(tr.doc.resolve($from.pos + 2)));
        return tr;
      }

      return exitChecklist(tr, ctx);
    }

    function adjustCursorPosition(tr) {
      var doc = tr.doc;
      var selection = tr.selection;
      var $from = selection.$from;

      if (!selection.empty) {
        return null;
      }

      var parent = $from.parent;
      if (!startsWithCheck(parent) || !inBulletListItem(doc, $from.pos)) {
        return null;
      }

      var checkSize = parent.firstChild.nodeSize;
      var minPos = hasSpaceAfterCheck(parent) ? checkSize + 1 : checkSize;

      if ($from.parentOffset < minPos) {
        tr.setSelection(selection.constructor.near(doc.resolve($from.start() + minPos)));
        return tr;
      }

      return null;
    }

    return [
      new Plugin({
        props: {
          handleClickOn: function (view, pos, node, nodePos) {
            if (node.type.name === "check") {
              view.dispatch(view.state.tr.setNodeMarkup(nodePos, null, { checked: !node.attrs.checked }));
              return true;
            }
            return false;
          },

          handleKeyDown: function (view, event) {
            if (event.key !== "Backspace" && event.key !== "ArrowLeft") {
              return false;
            }

            var state = view.state;
            var dispatch = view.dispatch;
            var selection = state.selection;
            var $from = selection.$from;

            // Right after a checklist item's check and space only.
            if (
              !selection.empty ||
              $from.parentOffset !== 2 ||
              !startsWithCheck($from.parent) ||
              !inBulletListItem(state.doc, $from.pos)
            ) {
              return false;
            }

            if (event.key === "Backspace") {
              var ctx = findBulletListContext($from);

              var checkStart = $from.start();
              var tr = state.tr.delete(checkStart, checkStart + 2);

              if (ctx) {
                var listItemPos = tr.mapping.map($from.before(ctx.listItemDepth));
                var $listItem = tr.doc.resolve(listItemPos);

                if ($listItem.nodeBefore && $listItem.nodeBefore.type === listItemType) {
                  tr = tr.join(listItemPos, 2);
                }
              }

              dispatch(tr);
              return true;
            }

            var beforeTextblock = $from.before();
            if (beforeTextblock > 0) {
              dispatch(state.tr.setSelection(selection.constructor.near(state.doc.resolve(beforeTextblock), -1)));
              return true;
            }

            return false;
          },
        },

        appendTransaction: function (transactions, oldState, newState) {
          var isFullReplace = transactions.some(function (t) {
            return t.steps.length === 1 && t.steps[0].from === 0 && t.steps[0].to === oldState.doc.content.size;
          });
          if (isFullReplace) {
            return null;
          }

          var tr = newState.tr;
          ensureSpaceAfterChecks(tr, oldState, newState);

          var continued = handleChecklistContinuation(tr, transactions);
          if (continued) {
            return continued;
          }
          var adjusted = adjustCursorPosition(tr);
          if (adjusted) {
            return adjusted;
          }
          return tr.docChanged ? tr : null;
        },
      }),

      // has-checkbox on checklist items
      new Plugin({
        props: {
          decorations: function (state) {
            var decorations = [];

            state.doc.descendants(function (node, pos, parent) {
              if (
                node.type === listItemType &&
                parent &&
                parent.type === bulletListType &&
                startsWithCheck(node.firstChild)
              ) {
                decorations.push(Decoration.node(pos, pos + node.nodeSize, { class: "has-checkbox" }));
              }
            });

            return DecorationSet.create(state.doc, decorations);
          },
        },
      }),
    ];
  },

  commands: function (params) {
    var schema = params.schema;
    var pmSchemaList = params.pmSchemaList;
    var checkType = schema.nodes.check;
    var bulletListType = schema.nodes.bullet_list;
    var orderedListType = schema.nodes.ordered_list;
    var listItemType = schema.nodes.list_item;

    function listItemHasCheck(listItem) {
      var p = listItem.firstChild;
      return !!(p && p.isTextblock && p.firstChild && p.firstChild.type === checkType);
    }

    function findListContext(state) {
      var $from = state.selection.$from;
      var $to = state.selection.$to;
      for (var depth = $from.depth; depth > 0; depth--) {
        if ($from.node(depth).type === listItemType) {
          var list = $from.node(depth - 1);
          if (list && (list.type === bulletListType || list.type === orderedListType)) {
            return {
              listDepth: depth - 1,
              listItemDepth: depth,
              listType: list.type,
              list: list,
              listStart: $from.before(depth - 1),
              $from: $from,
              $to: $to,
            };
          }
        }
      }
      return null;
    }

    function forEachSelectedItem(ctx, callback) {
      var collapsed = ctx.$from.pos === ctx.$to.pos;

      ctx.list.forEach(function (item, offset) {
        var itemStart = ctx.listStart + 1 + offset;
        var itemEnd = itemStart + item.nodeSize;
        var inSelection = collapsed
          ? ctx.$from.pos >= itemStart && ctx.$from.pos <= itemEnd
          : !(itemEnd <= ctx.$from.pos || itemStart >= ctx.$to.pos);

        if (inSelection) {
          callback(item, itemStart);
        }
      });
    }

    function hasCheckInSelection(ctx) {
      if (!ctx || ctx.listType !== bulletListType) {
        return false;
      }
      var found = false;
      forEachSelectedItem(ctx, function (item) {
        if (listItemHasCheck(item)) {
          found = true;
        }
      });
      return found;
    }

    function removeChecksFromSelection(state, ctx) {
      var toDelete = [];
      forEachSelectedItem(ctx, function (item, itemStart) {
        if (listItemHasCheck(item)) {
          var textblock = item.firstChild;
          var checkSize = textblock.firstChild.nodeSize;
          var second = textblock.childCount > 1 ? textblock.child(1) : null;
          var hasSpace = !!(second && second.isText && second.text && second.text[0] === " ");
          toDelete.push({
            from: itemStart + 2,
            to: itemStart + 2 + checkSize + (hasSpace ? 1 : 0),
          });
        }
      });

      var tr = state.tr;
      for (var i = toDelete.length - 1; i >= 0; i--) {
        tr = tr.delete(toDelete[i].from, toDelete[i].to);
      }
      return tr;
    }

    function addChecksToSelection(state, ctx) {
      var toInsert = [];
      forEachSelectedItem(ctx, function (item, itemStart) {
        if (!listItemHasCheck(item) && item.firstChild && item.firstChild.isTextblock) {
          toInsert.push(itemStart + 2);
        }
      });

      var tr = state.tr;
      var offset = 0;
      toInsert.forEach(function (pos) {
        var check = checkType.create({ checked: false });
        var space = schema.text(" ");
        tr = tr.insert(pos + offset, [check, space]);
        offset += check.nodeSize + space.nodeSize;
      });
      return tr;
    }

    return {
      toggleBulletList: function () {
        return function (state, dispatch) {
          var ctx = findListContext(state);
          if (hasCheckInSelection(ctx)) {
            if (dispatch) {
              dispatch(removeChecksFromSelection(state, ctx));
            }
            return true;
          }
          return false;
        };
      },

      toggleOrderedList: function () {
        return function (state, dispatch, view) {
          var ctx = findListContext(state);
          if (!hasCheckInSelection(ctx)) {
            return false;
          }

          if (!dispatch) {
            return true;
          }

          var liftListItem = pmSchemaList && pmSchemaList.liftListItem;
          var wrapInList = pmSchemaList && pmSchemaList.wrapInList;
          if (!liftListItem || !wrapInList) {
            return false;
          }

          dispatch(removeChecksFromSelection(state, ctx));
          if (view) {
            liftListItem(listItemType)(view.state, dispatch);
            wrapInList(orderedListType)(view.state, dispatch);
          }
          return true;
        };
      },

      toggleChecklist: function () {
        return function (state, dispatch, view) {
          var ctx = findListContext(state);
          var wrapInList = pmSchemaList && pmSchemaList.wrapInList;
          var liftListItem = pmSchemaList && pmSchemaList.liftListItem;
          var newCtx;

          if (hasCheckInSelection(ctx)) {
            if (!dispatch || !liftListItem) {
              return !!liftListItem;
            }
            dispatch(removeChecksFromSelection(state, ctx));
            if (view) {
              liftListItem(listItemType)(view.state, dispatch);
            }
            return true;
          }

          if (ctx && ctx.listType === bulletListType) {
            if (dispatch) {
              dispatch(addChecksToSelection(state, ctx));
            }
            return true;
          }

          if (ctx && ctx.listType === orderedListType) {
            if (!dispatch || !liftListItem || !wrapInList) {
              return !!(liftListItem && wrapInList);
            }
            liftListItem(listItemType)(state, dispatch);
            if (view) {
              wrapInList(bulletListType)(view.state, dispatch);
              newCtx = findListContext(view.state);
              if (newCtx) {
                dispatch(addChecksToSelection(view.state, newCtx));
              }
            }
            return true;
          }

          if (!wrapInList) {
            return false;
          }
          if (!dispatch) {
            return wrapInList(bulletListType)(state, undefined);
          }

          wrapInList(bulletListType)(state, dispatch);
          if (view) {
            newCtx = findListContext(view.state);
            if (newCtx) {
              dispatch(addChecksToSelection(view.state, newCtx));
            }
          }
          return true;
        };
      },
    };
  },

  state: function (params, viewState) {
    var schema = params.schema;
    var $from = viewState.selection.$from;
    var inCheckList = false;

    for (var depth = $from.depth; depth > 0; depth--) {
      var node = $from.node(depth);
      if (node.type === schema.nodes.list_item) {
        var list = $from.node(depth - 1);
        if (list && list.type === schema.nodes.bullet_list) {
          var p = node.firstChild;
          inCheckList = !!(p && p.isTextblock && p.firstChild && p.firstChild.type === schema.nodes.check);
        }
        break;
      }
    }

    return {
      inCheckList: inCheckList,
      inBulletList: !inCheckList && params.utils.inNode(viewState, schema.nodes.bullet_list),
    };
  },
};
