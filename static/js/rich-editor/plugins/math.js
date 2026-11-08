// discourse-math's lib/rich-editor-extension.js: $...$ (or %...% for
// asciimath) and $$...$$ as math atoms (math_inline and math_block in the
// node tree) typeset as the posts are (discourse-math.js
// Discourse.renderMath), with an edit button that opens the Edit math
// modal (getContext().modal).
import { iconHtml } from "discourse/rich-editor/toolbar";

function escapeDelimiter(text, delimiter) {
  if (!text) {
    return "";
  }

  var result = "";

  for (var i = 0; i < text.length; i++) {
    var char = text[i];
    if (char === delimiter) {
      var backslashes = 0;
      var j = i - 1;
      while (j >= 0 && text[j] === "\\") {
        backslashes++;
        j--;
      }
      if (backslashes % 2 === 0) {
        result += "\\";
      }
    }
    result += char;
  }

  return result;
}

function MathNodeView(node, view, getPos, params) {
  var self = this;
  this.node = node;
  this.view = view;
  this.getPos = getPos;
  this.getContext = params.getContext;
  this.NodeSelection = params.pmState.NodeSelection;
  var i18n = this.getContext().i18n;

  this.openEditModal = function (event) {
    event.preventDefault();
    event.stopPropagation();

    self.getContext().modal.show("math-edit", {
      model: {
        initialText: self.node.attrs.text || "",
        isBlock: !self.node.isInline,
        mathType: self.node.attrs.mathType || "tex",
        onApply: function (text) {
          self.applyEdit(text);
        },
      },
    });
  };

  var isInline = node.isInline;
  this.dom = document.createElement(isInline ? "span" : "div");
  this.dom.classList.add("composer-math-node");

  this.editButton = document.createElement("button");
  this.editButton.type = "button";
  this.editButton.classList.add("btn-flat", "math-node-edit-button");
  this.editButton.setAttribute("contenteditable", "false");
  this.editButton.setAttribute("title", i18n("discourse_math.edit_math"));
  this.editButton.setAttribute("aria-label", i18n("discourse_math.edit_math"));
  this.editButton.innerHTML = iconHtml("pencil");
  this.editButton.addEventListener("click", this.openEditModal);

  this.content = document.createElement(isInline ? "span" : "div");
  this.content.classList.add("math-node-content");
  this.content.setAttribute("contenteditable", "false");

  this.dom.appendChild(this.editButton);
  this.dom.appendChild(this.content);

  this.syncContent();
  this.renderMath();
}

MathNodeView.prototype.update = function (node) {
  var contentChanged = node.attrs.text !== this.node.attrs.text || node.attrs.mathType !== this.node.attrs.mathType;
  this.node = node;

  if (contentChanged) {
    this.syncContent();
    this.renderMath();
  }

  return true;
};

MathNodeView.prototype.selectNode = function () {
  this.dom.classList.add("ProseMirror-selectednode");
};

MathNodeView.prototype.deselectNode = function () {
  this.dom.classList.remove("ProseMirror-selectednode");
};

MathNodeView.prototype.stopEvent = function (event) {
  return event.target instanceof Node ? this.editButton.contains(event.target) : false;
};

MathNodeView.prototype.ignoreMutation = function () {
  return true;
};

MathNodeView.prototype.destroy = function () {
  this.editButton.removeEventListener("click", this.openEditModal);
};

MathNodeView.prototype.syncContent = function () {
  var isAscii = this.node.isInline && this.node.attrs.mathType === "asciimath";

  this.content.classList.toggle("asciimath", isAscii);
  this.content.classList.toggle("math", !isAscii);
  this.content.textContent = this.node.attrs.text || "";
};

// renderMathInElement(dom, options, { force: true }); the page loads the
// renderer while discourse_math_enabled is on.
MathNodeView.prototype.renderMath = function () {
  if (window.Discourse && window.Discourse.renderMath) {
    window.Discourse.renderMath(this.dom, { force: true });
  }
};

MathNodeView.prototype.applyEdit = function (text) {
  var pos = this.getPos();
  var attrs = Object.assign({}, this.node.attrs, { text: text });
  var tr = this.view.state.tr.setNodeMarkup(pos, null, attrs);
  tr.setSelection(this.NodeSelection.create(tr.doc, pos));
  this.view.dispatch(tr);
};

function createMathNodeView(params) {
  return function (node, view, getPos) {
    return new MathNodeView(node, view, getPos, params);
  };
}

export default {
  nodeViews: {
    math_inline: createMathNodeView,
    math_block: createMathNodeView,
  },
  nodeSpec: {
    math_inline: {
      inline: true,
      group: "inline",
      atom: true,
      selectable: true,
      draggable: true,
      attrs: {
        text: { default: "" },
        mathType: { default: "tex" },
      },
      parseDOM: [
        {
          tag: "span.math",
          getAttrs: function (dom) {
            return { text: dom.textContent.trim(), mathType: "tex" };
          },
        },
        {
          tag: "span.asciimath",
          getAttrs: function (dom) {
            return { text: dom.textContent.trim(), mathType: "asciimath" };
          },
        },
      ],
      toDOM: function (node) {
        return ["span", { class: node.attrs.mathType === "asciimath" ? "asciimath" : "math" }, node.attrs.text];
      },
    },
    math_block: {
      group: "block",
      atom: true,
      selectable: true,
      defining: true,
      isolating: true,
      attrs: {
        text: { default: "" },
        mathType: { default: "tex" },
      },
      parseDOM: [
        {
          tag: "div.math",
          getAttrs: function (dom) {
            return { text: dom.textContent.trim(), mathType: "tex" };
          },
        },
      ],
      toDOM: function (node) {
        return ["div", { class: "math" }, node.attrs.text];
      },
    },
  },
  parse: {
    math_inline: {
      node: "math_inline",
      getAttrs: function (token) {
        return { text: token.content, mathType: (token.meta && token.meta.mathType) || "tex" };
      },
    },
    math_block: {
      node: "math_block",
      getAttrs: function (token) {
        return { text: token.content, mathType: (token.meta && token.meta.mathType) || "tex" };
      },
    },
  },
  serializeNode: function (params) {
    var isBoundary = params.utils.isBoundary;
    return {
      math_inline: function (state, node, parent, index) {
        state.flushClose();
        if (!isBoundary(state.out, state.out.length - 1)) {
          state.write(" ");
        }

        var delimiter = node.attrs.mathType === "asciimath" ? "%" : "$";
        state.write(delimiter + escapeDelimiter(node.attrs.text || "", delimiter) + delimiter);

        var nextSibling = parent.childCount > index + 1 ? parent.child(index + 1) : null;
        if (nextSibling && nextSibling.isText && !isBoundary(nextSibling.text, 0)) {
          state.write(" ");
        }
      },
      math_block: function (state, node) {
        state.ensureNewLine();
        state.write("$$\n");
        state.write(escapeDelimiter(node.attrs.text || "", "$"));
        state.write("\n$$\n\n");
      },
    };
  },
};
