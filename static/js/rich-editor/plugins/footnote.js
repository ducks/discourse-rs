// footnote's lib/rich-editor-extension.js: a footnote reference
// (footnote_ref in the node tree) as an atom holding the note's blocks,
// which footnote_open fills from the list after the document; selecting it
// opens the note in an editor of its own (prosemirror.net's footnote
// example). Written back as ^[note] when it is one paragraph, else as
// [^n] with the notes after the document.
function createFootnoteNodeView(params) {
  var EditorView = params.pmView.EditorView;
  var EditorState = params.pmState.EditorState;
  var StepMap = params.pmTransform.StepMap;

  function FootnoteNodeView(node, view, getPos) {
    this.node = node;
    this.outerView = view;
    this.getPos = getPos;

    this.dom = document.createElement("div");
    this.dom.className = "footnote";
    this.innerView = null;
  }

  FootnoteNodeView.prototype.selectNode = function () {
    this.dom.classList.add("ProseMirror-selectednode");
    if (!this.innerView) {
      this.open();
    }

    if (this.innerView) {
      this.innerView.dom.focus();
    }
  };

  FootnoteNodeView.prototype.deselectNode = function () {
    this.dom.classList.remove("ProseMirror-selectednode");
    if (this.innerView) {
      this.close();
    }
  };

  FootnoteNodeView.prototype.open = function () {
    var self = this;
    var tooltip = this.dom.appendChild(document.createElement("div"));
    tooltip.style.setProperty("--footnote-counter", '"' + this.footnoteCounterValue() + '"');
    tooltip.className = "footnote-tooltip";

    this.innerView = new EditorView(tooltip, {
      state: EditorState.create({
        doc: this.node,
        plugins: this.outerView.state.plugins.filter(function (plugin) {
          return !/^(placeholder|trailing-paragraph)\$.*/.test(plugin.key);
        }),
      }),
      dispatchTransaction: this.dispatchInner.bind(this),
      handleDOMEvents: {
        mousedown: function () {
          // The whole footnote is node-selected (and so DOM-selected)
          // while the outer editor has the focus.
          if (self.outerView.hasFocus()) {
            self.innerView.focus();
          }
        },
      },
    });
  };

  FootnoteNodeView.prototype.close = function () {
    this.innerView.destroy();
    this.innerView = null;
    this.dom.textContent = "";
  };

  FootnoteNodeView.prototype.dispatchInner = function (tr) {
    var applied = this.innerView.state.applyTransaction(tr);
    this.innerView.updateState(applied.state);

    if (!tr.getMeta("fromOutside")) {
      var outerTr = this.outerView.state.tr;
      var offsetMap = StepMap.offset(this.getPos() + 1);
      applied.transactions.forEach(function (transaction) {
        transaction.steps.forEach(function (step) {
          outerTr.step(step.map(offsetMap));
        });
      });
      if (outerTr.docChanged) {
        this.outerView.dispatch(outerTr);
      }
    }
  };

  FootnoteNodeView.prototype.update = function (node) {
    if (!node.sameMarkup(this.node)) {
      return false;
    }
    this.node = node;
    if (this.innerView) {
      var state = this.innerView.state;
      var start = node.content.findDiffStart(state.doc.content);
      if (start !== null && start !== undefined) {
        var end = node.content.findDiffEnd(state.doc.content);
        var endA = end.a;
        var endB = end.b;
        var overlap = start - Math.min(endA, endB);
        if (overlap > 0) {
          endA += overlap;
          endB += overlap;
        }
        this.innerView.dispatch(state.tr.replace(start, endB, node.slice(start, endA)).setMeta("fromOutside", true));
      }
    }
    return true;
  };

  FootnoteNodeView.prototype.destroy = function () {
    if (this.innerView) {
      this.close();
    }
  };

  FootnoteNodeView.prototype.stopEvent = function (event) {
    return !!(this.innerView && this.innerView.dom.contains(event.target));
  };

  FootnoteNodeView.prototype.ignoreMutation = function () {
    return true;
  };

  FootnoteNodeView.prototype.footnoteCounterValue = function () {
    var editor = this.dom.closest(".ProseMirror");
    var footnotes = editor ? editor.querySelectorAll(".footnote") : [];

    return Array.prototype.indexOf.call(footnotes, this.dom) + 1;
  };

  return function (node, view, getPos) {
    return new FootnoteNodeView(node, view, getPos);
  };
}

export default {
  nodeViews: { footnote: createFootnoteNodeView },
  nodeSpec: {
    footnote: {
      attrs: { id: {} },
      group: "inline",
      content: "block*",
      inline: true,
      atom: true,
      draggable: false,
      parseDOM: [{ tag: "div.footnote" }],
      toDOM: function () {
        return ["div", { class: "footnote" }, 0];
      },
    },
  },
  parse: function (params) {
    var Slice = params.pmModel.Slice;
    var Fragment = params.pmModel.Fragment;

    return {
      footnote_ref: {
        node: "footnote",
        getAttrs: function (token) {
          return { id: token.meta.id };
        },
      },
      footnote_block: { ignore: true },
      footnote_open: function (state, token, tokens, i) {
        // footnote_open is at the root
        var doc = state.top();

        var id = token.meta.id;
        var innerTokens = tokens.slice(i + 1, tokens.length - 1);
        var footnoteCloseIndex = innerTokens.findIndex(function (t) {
          return t.type === "footnote_close";
        });
        innerTokens = innerTokens.slice(0, footnoteCloseIndex);

        doc.content.forEach(function (node, pos) {
          var replacements = [];
          node.descendants(function (child, childPos) {
            if (child.type.name !== "footnote" || child.attrs.id !== id) {
              return;
            }

            // The note's tokens parsed with the footnote as their parent,
            // then the stack as it was.
            state.stack = [];
            state.openNode(state.schema.nodes.footnote);
            state.parseTokens(innerTokens);
            var footnote = state.closeNode();
            state.stack = [doc];

            var slice = new Slice(Fragment.from(footnote), 0, 0);
            replacements.push({ from: childPos, to: childPos + 2, slice: slice });
          });

          replacements.forEach(function (r) {
            doc.content[pos] = doc.content[pos].replace(r.from, r.to, r.slice);
          });
        });

        // the note's tokens and its footnote_close
        tokens.splice(i + 1, innerTokens.length + 1);
      },
      footnote_anchor: { ignore: true, noCloseToken: true },
    };
  },
  serializeNode: {
    footnote: function (state, node) {
      if (node.content.content.length === 1 && node.content.firstChild.type.name === "paragraph") {
        state.write("^[");
        state.renderContent(node.content.firstChild);
        state.write("]");
      } else {
        var contents = (state.footnoteContents = state.footnoteContents || []);
        contents.push(node.content);
        state.write("[^" + contents.length + "]");
      }
    },
    afterSerialize: function (state) {
      var contents = state.footnoteContents;

      if (!contents) {
        return;
      }

      for (var i = 0; i < contents.length; i++) {
        var oldDelim = state.delim;
        state.write("[^" + (i + 1) + "]: ");
        state.delim += "    ";
        state.renderContent(contents[i]);
        state.delim = oldDelim;
      }
    },
  },
  inputRules: function (params) {
    var NodeSelection = params.pmState.NodeSelection;
    return [
      {
        match: /\^\[(.*?)]$/,
        handler: function (state, match, start, end) {
          var content = state.doc.slice(start + 2, end).content;
          var paragraph = state.schema.nodes.paragraph.create(null, content);
          var footnote = state.schema.nodes.footnote.create(null, paragraph);

          return state.tr.replaceWith(start, end, footnote);
        },
      },
      {
        match: /\[\^\d+]$/,
        handler: function (state, match, start, end) {
          var paragraph = state.schema.nodes.paragraph.create();
          var footnote = state.schema.nodes.footnote.create(null, paragraph);

          var tr = state.tr.replaceWith(start, end, footnote);
          tr.setSelection(NodeSelection.create(tr.doc, start));
          return tr;
        },
      },
    ];
  },
};
