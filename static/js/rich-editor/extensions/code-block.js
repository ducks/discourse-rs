// extensions/code-block.js: code blocks highlighted as they are edited
// (prosemirror-highlightjs over the page's highlight.js, the instance
// static/js/highlight-syntax.js loads for cooked posts), a language
// select on each, Tab and Shift-Tab indenting the selected lines, the
// formatCode command, and pasted code (line wrappers, the language a class
// names) kept as one block.
import { setBlockType, toggleMark } from "prosemirror-commands";
import { highlightPlugin } from "prosemirror-highlightjs";
import { schema as markdownSchema } from "prosemirror-markdown";
import { TextSelection } from "prosemirror-state";

var INDENT = "  ";

var PLAIN_TEXT_LANGS = ["auto", "text", "pre", "plain"];

// the loaded instance, for the language selects
var hljs;

function selectedCodeLines(state) {
  var selection = state.selection;
  if (
    !(selection instanceof TextSelection) ||
    selection.empty ||
    selection.$from.parent !== selection.$to.parent ||
    !selection.$from.parent.type.spec.code
  ) {
    return;
  }

  var codeBlock = selection.$from.parent;
  var blockStart = selection.$from.start();
  var from = selection.from - blockStart;
  var to = selection.to - blockStart;
  var text = codeBlock.textContent;
  var firstLineStart = text.lastIndexOf("\n", from - 1) + 1;
  var effectiveTo = text[to - 1] === "\n" ? to - 1 : to;
  var lineStarts = [firstLineStart];

  var nextLineStart = text.indexOf("\n", firstLineStart) + 1;
  while (nextLineStart > 0 && nextLineStart <= effectiveTo) {
    lineStarts.push(nextLineStart);
    nextLineStart = text.indexOf("\n", nextLineStart) + 1;
  }

  return { blockStart: blockStart, lineStarts: lineStarts, text: text };
}

function indentCodeBlock(outdent) {
  return function (state, dispatch) {
    var selectedLines = selectedCodeLines(state);
    if (!selectedLines) {
      return false;
    }

    if (!dispatch) {
      return true;
    }

    var tr = state.tr;

    selectedLines.lineStarts.reverse().forEach(function (lineStart) {
      var pos = selectedLines.blockStart + lineStart;
      if (outdent) {
        var line = selectedLines.text.slice(lineStart);
        var spaces = line.startsWith(INDENT) ? INDENT.length : Number(line.startsWith(" "));
        if (spaces) {
          tr.delete(pos, pos + spaces);
        }
      } else {
        tr.insertText(INDENT, pos);
      }
    });

    if (tr.docChanged) {
      tr.setSelection(state.selection.map(tr.doc, tr.mapping));
      dispatch(tr.scrollIntoView());
    }

    return true;
  };
}

class CodeBlockWithLangSelectorNodeView {
  constructor(node, view, getPos) {
    this.node = node;
    this.view = view;
    this.getPos = getPos;
    this.selectAdded = false;

    var code = document.createElement("code");
    var pre = document.createElement("pre");
    pre.appendChild(code);

    this.dom = pre;
    this.contentDOM = code;

    this.appendSelect();
  }

  changeListener(e) {
    this.view.dispatch(this.view.state.tr.setNodeMarkup(this.getPos(), null, { params: e.target.value }));

    if (e.target.firstChild.textContent) {
      e.target.firstChild.textContent = "";
    }
  }

  appendSelect() {
    var self = this;
    if (!hljs || this.selectAdded) {
      return;
    }

    this.selectAdded = true;

    var select = document.createElement("select");
    select.contentEditable = false;
    select.addEventListener("change", function (e) {
      self.changeListener(e);
    });
    select.classList.add("code-language-select");

    var languages = hljs.listLanguages().sort(function (a, b) {
      return a.localeCompare(b);
    });

    var empty = document.createElement("option");
    empty.textContent = languages.indexOf(this.node.attrs.params) >= 0 ? "" : this.node.attrs.params;
    select.appendChild(empty);

    languages.forEach(function (lang) {
      var option = document.createElement("option");
      option.textContent = lang;
      option.selected = lang === self.node.attrs.params;
      select.appendChild(option);
    });

    this.dom.appendChild(select);
  }

  update(node) {
    this.appendSelect();

    return node.type === this.node.type;
  }
}

function convertCodeBlockToParagraphs(schema) {
  return function (editorState, dispatch) {
    var codeBlock = editorState.selection.$from.parent;
    var codeText = codeBlock.textContent;

    // a paragraph per \n\n
    var paragraphs = codeText.split("\n\n").filter(function (text) {
      return text.trim() !== "";
    });

    if (dispatch) {
      var startPos = editorState.selection.$from.start() - 1;
      var endPos = editorState.selection.$from.end() + 1;

      var paragraphNodes = paragraphs.map(function (text) {
        return schema.nodes.paragraph.create(null, schema.text(text));
      });

      var tr = editorState.tr.replaceWith(startPos, endPos, paragraphNodes);

      // the paragraphs selected, to toggle back and forth
      var newEndPos =
        startPos +
        paragraphNodes.reduce(function (size, node) {
          return size + node.nodeSize;
        }, 0);
      tr.setSelection(TextSelection.create(tr.doc, startPos, newEndPos));

      dispatch(tr);
    }
    return true;
  };
}

function isBlockLevelSelection(selection) {
  var hasMultipleBlocks =
    selection.$from.parent !== selection.$to.parent || selection.$from.parent.type.name === "doc";

  var isFullBlockSelection =
    selection.$from.parent === selection.$to.parent &&
    selection.from === selection.$from.start() - 1 &&
    selection.to === selection.$to.end() + 1;

  return hasMultipleBlocks || isFullBlockSelection;
}

var LANGUAGE_CLASS = /^lang(?:uage)?-(.+)$/;

function codeBlockLanguage() {
  for (var i = 0; i < arguments.length; i++) {
    var element = arguments[i];
    if (!element || !element.classList) {
      continue;
    }
    for (var j = 0; j < element.classList.length; j++) {
      var match = element.classList[j].match(LANGUAGE_CLASS);
      var language = match && match[1];
      // `auto` asks for detection; it is no language
      if (language && language !== "auto") {
        return language;
      }
    }
  }
}

export function normalizeCodeBlocks(doc) {
  var changed = false;

  doc.querySelectorAll("pre > code").forEach(function (code) {
    var pre = code.parentElement;

    if (!pre.hasAttribute("data-params")) {
      var language = codeBlockLanguage(code, pre, pre.parentElement);
      if (language) {
        pre.setAttribute("data-params", language);
        changed = true;
      }
    }

    var lines = Array.prototype.slice.call(code.childNodes);
    if (
      !lines.length ||
      !lines.every(function (line) {
        return line.nodeName === "DIV" && line.lastChild && line.lastChild.nodeName === "BR";
      })
    ) {
      return;
    }

    // The last break ends the last line; kept, it would add a blank one.
    lines[lines.length - 1].lastChild.remove();

    // Each line has its break already; block wrappers would split the code.
    lines.forEach(function (line) {
      line.replaceWith.apply(line, Array.prototype.slice.call(line.childNodes));
    });
    changed = true;
  });

  return changed;
}

function convertSelectionToCodeBlock(schema) {
  return function (editorState, dispatch) {
    var from = editorState.selection.from;
    var to = editorState.selection.to;
    // the text with its blocks apart
    var textContent = editorState.doc.textBetween(from, to, "\n\n", "\n") || "code";

    var codeBlock = schema.nodes.code_block.create({}, schema.text(textContent));

    if (dispatch) {
      var tr = editorState.tr.replaceWith(from, to, codeBlock);
      var codeBlockStart = from + 1;
      var codeBlockEnd = codeBlockStart + codeBlock.content.size;
      tr.setSelection(TextSelection.create(tr.doc, codeBlockStart, codeBlockEnd));
      dispatch(tr);
    }
    return true;
  };
}

export default {
  nodeSpec: {
    code_block: Object.assign({ createGapCursor: true }, markdownSchema.nodes.code_block.spec),
  },
  nodeViews: { code_block: CodeBlockWithLangSelectorNodeView },
  transformParsedHTML: normalizeCodeBlocks,
  keymap: function () {
    return {
      Tab: indentCodeBlock(),
      "Shift-Tab": indentCodeBlock(true),
    };
  },
  commands: function (params) {
    var schema = params.schema;
    return {
      formatCode: function () {
        return function (state, dispatch) {
          var selection = state.selection;

          // In a code block: back to paragraphs
          if (selection.$from.parent.type === schema.nodes.code_block) {
            return convertCodeBlockToParagraphs(schema)(state, dispatch);
          }

          if (selection.empty) {
            var isEmptyBlock = selection.$from.parent.content.size === 0;
            var command = isEmptyBlock ? setBlockType(schema.nodes.code_block) : toggleMark(schema.marks.code);
            return command(state, dispatch);
          }

          // Several blocks, or a whole one
          if (isBlockLevelSelection(selection)) {
            return convertSelectionToCodeBlock(schema)(state, dispatch);
          }

          return toggleMark(schema.marks.code)(state, dispatch);
        };
      },
    };
  },
  plugins: [
    function (params) {
      return new params.pmState.Plugin({
        props: {
          transformPastedHTML: function (html) {
            var doc = new DOMParser().parseFromString(html, "text/html");
            return normalizeCodeBlocks(doc) ? doc.body.innerHTML : html;
          },
        },
      });
    },
    async function (params) {
      var siteSettings = params.getContext().siteSettings || {};

      // Detection can hang the browser (b-kelly/prosemirror-highlightjs#21)
      var defaultCodeLang = siteSettings.default_code_lang;
      if (!defaultCodeLang || PLAIN_TEXT_LANGS.indexOf(defaultCodeLang) >= 0) {
        defaultCodeLang = "text";
      }

      hljs = await window.Discourse.highlightSyntax.ensureHighlightJs();
      return highlightPlugin(hljs, ["code_block", "html_block", "preview_source"], function (node) {
        return node.attrs.params || (node.type.name === "code_block" ? defaultCodeLang : "text");
      });
    },
  ],
};
