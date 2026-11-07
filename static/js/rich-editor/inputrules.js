// The editor's input rules (static/prosemirror/core/inputrules.js): with
// the default schema, `> ` quotes, `1. ` and `- ` lists, ``` and four
// spaces for code, `# ` headings, **, __, * and _ marks and ---, ___ or ***
// for a rule; then each extension's inputRules. None fires right after a
// backtick, and the block rules don't fire inside inline code.
import { InputRule, inputRules, textblockTypeInputRule, wrappingInputRule } from "prosemirror-inputrules";
import { TextSelection } from "prosemirror-state";
import { markInputRule } from "discourse/rich-editor/plugin-utils";

export function buildInputRules(extensions, params, includeDefault) {
  if (includeDefault === undefined) {
    includeDefault = true;
  }
  var rules = [];

  if (includeDefault) {
    var schema = params.schema;

    // The two groups as the single group markInputRule expects.
    var getAttrs = function (match) {
      return {
        match: [match[0].replace(match[1], ""), match[2]],
        start: match[1].length,
      };
    };

    var blockRules = [
      wrappingInputRule(/^\s*>\s$/, schema.nodes.blockquote),
      orderedListRule(schema.nodes.ordered_list),
      bulletListRule(schema.nodes.bullet_list),
      textblockTypeInputRule(/^```$/, schema.nodes.code_block),
      textblockTypeInputRule(/^ {4}$/, schema.nodes.code_block),
      headingRule(schema.nodes.heading, 6),
    ].map(function (rule) {
      rule.inCodeMark = false;
      return rule;
    });

    var defaultRules = blockRules.concat([
      markInputRule(/\*\*([^*]+)\*\*$/, schema.marks.strong),
      markInputRule(/(^|\s)__([^_]+)__$/, schema.marks.strong, getAttrs),
      markInputRule(/(^|[^*])\*([^*]+)\*$/, schema.marks.em, getAttrs),
      markInputRule(/(^|\s)_([^_]+)_$/, schema.marks.em, getAttrs),
      // `---`, `___ `, `***`. The en/em-dash variants are kept for content
      // that already has those characters (pasted, say), since the editor
      // doesn't turn `--`/`---` into dashes as you type.
      new InputRule(/^(---|\u2013-|\u2014-|___\s|\*\*\*\s)$/, horizontalRuleHandler, { inCodeMark: false }),
    ]);

    defaultRules.forEach(function (rule) {
      rules.push(processInputRule(rule, params));
    });
  }

  extractInputRules(extensions, params).forEach(function (rule) {
    rules.push(rule);
  });

  return inputRules({ rules: flat(rules) });
}

function flat(list) {
  return list.reduce(function (all, item) {
    return all.concat(item instanceof Array ? flat(item) : [item]);
  }, []);
}

function extractInputRules(extensions, params) {
  var rules = [];
  extensions.forEach(function (extension) {
    if (extension.inputRules) {
      rules.push(processInputRule(extension.inputRules, params));
    }
  });
  return flat(rules);
}

function processInputRule(inputRule, params) {
  if (inputRule instanceof Array) {
    return inputRule.map(function (rule) {
      return processInputRule(rule, params);
    });
  }

  if (inputRule instanceof Function) {
    return processInputRule(inputRule(params));
  }

  if (
    inputRule instanceof InputRule ||
    (inputRule.match instanceof RegExp && inputRule.handler instanceof Function)
  ) {
    var inCodeMark, inCode, undoable;

    if (inputRule instanceof InputRule) {
      // InputRule instances keep these as properties.
      inCodeMark = firstDefined(inputRule.inCodeMark, inputRule.inCode, false);
      inCode = firstDefined(inputRule.inCode, false);
      undoable = firstDefined(inputRule.undoable, true);
    } else {
      // Plain objects keep them under options.
      var options = inputRule.options || {};
      inCodeMark = firstDefined(options.inCodeMark, options.inCode, false);
      inCode = firstDefined(options.inCode, false);
      undoable = firstDefined(options.undoable, true);
    }

    return new InputRule(inputRule.match, wrapHandlerWithBacktickCheck(inputRule.handler), {
      undoable: undoable,
      inCode: inCode,
      inCodeMark: inCodeMark,
    });
  }

  throw new Error("Input rule must have a match regex and a handler function");
}

function firstDefined() {
  for (var i = 0; i < arguments.length; i++) {
    if (arguments[i] !== undefined && arguments[i] !== null) {
      return arguments[i];
    }
  }
}

function hasBacktickBefore(state, pos) {
  return pos > 0 && state.doc.textBetween(pos - 1, pos, "\n", "\n") === "`";
}

function wrapHandlerWithBacktickCheck(handler) {
  return function (state, match, start, end) {
    if (hasBacktickBefore(state, start)) {
      return null;
    }

    // Two-group patterns like (^|\W)(:emoji:) or (^|\W)(@mention): also a
    // backtick before the content after the boundary group.
    if (match[1] && match[2] && hasBacktickBefore(state, start + match[1].length)) {
      return null;
    }

    return handler(state, match, start, end);
  };
}

function orderedListRule(nodeType) {
  return wrappingInputRule(
    /^(\d+)\.\s$/,
    nodeType,
    function (match) {
      return { order: +match[1] };
    },
    function (match, node) {
      return node.childCount + node.attrs.order === +match[1];
    }
  );
}

function bulletListRule(nodeType) {
  return wrappingInputRule(/^\s*([-+*])\s$/, nodeType);
}

function headingRule(nodeType, maxLevel) {
  return textblockTypeInputRule(new RegExp("^(#{1," + maxLevel + "})\\s$"), nodeType, function (match) {
    return { level: match[1].length };
  });
}

function horizontalRuleHandler(state, match, start, end) {
  var tr = state.tr;
  tr.replaceRangeWith(start, end, [state.schema.nodes.horizontal_rule.create(), state.schema.nodes.paragraph.create()]);
  tr.setSelection(TextSelection.create(tr.doc, start + 1));
  return tr;
}
