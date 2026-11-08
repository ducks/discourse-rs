// Markdown into an editor document (static/prosemirror/core/parser.js).
// The reference tokenizes with JS markdown-it and hands the tokens to
// prosemirror-markdown's MarkdownParser; the port parses with its markdown
// crate (crates/markdown, as WebAssembly), whose export returns the parse as
// a node tree in JSON (docs/rich-editor.md, "The node tree"). tokensFromTree
// flattens that tree back into the token stream markdown-it would have
// produced, so MarkdownParser and every extension's parse spec, written
// against markdown-it tokens, work as they do in the reference.
import { defaultMarkdownParser, MarkdownParser } from "prosemirror-markdown";
import { getEngine } from "discourse/rich-editor/engine";

export function UnsupportedTokenError(message) {
  this.name = "UnsupportedTokenError";
  this.message = message || "unsupported token";
  this.stack = new Error(this.message).stack;
}
UnsupportedTokenError.prototype = Object.create(Error.prototype);
UnsupportedTokenError.prototype.constructor = UnsupportedTokenError;

// A tree that breaks the contract: a bug in the export, not in the post.
export function NodeTreeError(message) {
  this.name = "NodeTreeError";
  this.message = message;
  this.stack = new Error(message).stack;
}
NodeTreeError.prototype = Object.create(Error.prototype);
NodeTreeError.prototype.constructor = NodeTreeError;

// markdown-it's Token (lib/token.mjs): the fields and attr methods the
// parse specs use.
export function Token(type, tag, nesting) {
  this.type = type;
  this.tag = tag;
  this.attrs = null;
  this.map = null;
  this.nesting = nesting;
  this.level = 0;
  this.children = null;
  this.content = "";
  this.markup = "";
  this.info = "";
  this.meta = null;
  this.block = false;
  this.hidden = false;
}

Token.prototype.attrIndex = function (name) {
  if (!this.attrs) {
    return -1;
  }
  for (var i = 0; i < this.attrs.length; i++) {
    if (this.attrs[i][0] === name) {
      return i;
    }
  }
  return -1;
};

Token.prototype.attrPush = function (attrData) {
  if (this.attrs) {
    this.attrs.push(attrData);
  } else {
    this.attrs = [attrData];
  }
};

Token.prototype.attrSet = function (name, value) {
  var idx = this.attrIndex(name);
  if (idx < 0) {
    this.attrPush([name, value]);
  } else {
    this.attrs[idx] = [name, value];
  }
};

Token.prototype.attrGet = function (name) {
  var idx = this.attrIndex(name);
  return idx >= 0 ? this.attrs[idx][1] : null;
};

Token.prototype.attrJoin = function (name, value) {
  var idx = this.attrIndex(name);
  if (idx < 0) {
    this.attrPush([name, value]);
  } else {
    this.attrs[idx][1] = this.attrs[idx][1] + " " + value;
  }
};

function str(node, key) {
  var value = node[key];
  if (value === undefined || value === null) {
    return "";
  }
  if (typeof value !== "string") {
    throw new NodeTreeError(node.type + "." + key + " is not a string");
  }
  return value;
}

function attrsOf(node) {
  if (node.attrs === undefined || node.attrs === null) {
    return null;
  }
  if (
    !Array.isArray(node.attrs) ||
    !node.attrs.every(function (pair) {
      return (
        Array.isArray(pair) &&
        pair.length === 2 &&
        typeof pair[0] === "string" &&
        typeof pair[1] === "string"
      );
    })
  ) {
    throw new NodeTreeError(node.type + ".attrs is not a list of [name, value] strings");
  }
  return node.attrs.map(function (pair) {
    return [pair[0], pair[1]];
  });
}

function fill(token, node, level, block) {
  token.attrs = attrsOf(node);
  token.map = Array.isArray(node.map) ? node.map.slice() : null;
  token.level = level;
  token.markup = str(node, "markup");
  token.info = str(node, "info");
  token.meta = node.meta === undefined ? null : node.meta;
  token.block = block;
  token.hidden = node.hidden === true;
  // A rule's own properties (the poll's poll_attrs).
  if (node.fields !== undefined && node.fields !== null) {
    if (typeof node.fields !== "object" || Array.isArray(node.fields)) {
      throw new NodeTreeError(node.type + ".fields is not an object");
    }
    Object.keys(node.fields).forEach(function (name) {
      if (!(name in token)) {
        token[name] = node.fields[name];
      }
    });
  }
  return token;
}

function flatten(nodes, level, out, block) {
  if (!Array.isArray(nodes)) {
    throw new NodeTreeError("children is not a list");
  }
  nodes.forEach(function (node) {
    if (!node || typeof node.type !== "string" || node.type === "") {
      throw new NodeTreeError("a node without a type");
    }
    var tag = str(node, "tag");

    // A block's inline content: one token holding the inline tokens.
    if (node.type === "inline") {
      if (!block) {
        throw new NodeTreeError("inline inside inline content");
      }
      var inline = fill(new Token("inline", "", 0), node, level, true);
      inline.content = str(node, "content");
      inline.children = [];
      flatten(node.children || [], 0, inline.children, false);
      out.push(inline);
      return;
    }

    if (Array.isArray(node.children) && node.nesting !== 0) {
      var bare = node.bare === true;
      var open = fill(new Token(bare ? node.type : node.type + "_open", tag, 1), node, level, block);
      out.push(open);
      flatten(node.children, level + 1, out, block);
      var close = new Token(bare ? node.type : node.type + "_close", tag, -1);
      close.level = level;
      close.markup = open.markup;
      close.block = block;
      close.hidden = open.hidden;
      out.push(close);
      return;
    }

    var token = fill(new Token(node.type, tag, 0), node, level, block);
    token.content = str(node, "content");
    if (Array.isArray(node.children)) {
      token.children = [];
      flatten(node.children, 0, token.children, false);
    }
    out.push(token);
  });
  return out;
}

// The export's tree as markdown-it's token stream.
export function tokensFromTree(tree) {
  if (!tree || tree.type !== "doc" || !Array.isArray(tree.children)) {
    throw new NodeTreeError("the tree's root is not a doc with children");
  }
  return flatten(tree.children, 0, [], true);
}

// extensions' `parse` entries: parse specs by token name, or functions
// (state, token, tokens, i) handling a token themselves. Several functions
// for one token are tried in turn until one returns true. parseTree is the
// export: markdown in, the node tree out.
export default function Parser(extensions, params, includeDefault, parseTree) {
  if (includeDefault === undefined) {
    includeDefault = true;
  }
  this.params = params;
  this.parseTree = parseTree;
  this.multipleParseSpecs = {};

  this.parseTokens = includeDefault
    ? Object.assign({}, defaultMarkdownParser.tokens, {
        bbcode_b: { mark: "strong" },
        bbcode_i: { mark: "em" },
      })
    : {};

  this.postParseTokens = includeDefault
    ? {
        softbreak: function (state) {
          state.addNode(state.schema.nodes.hard_break);
        },
      }
    : {};

  var parsers = this.extractParsers(extensions);
  var self = this;
  Object.keys(parsers).forEach(function (key) {
    var value = parsers[key];
    // Not a ParseSpec
    if (typeof value === "function") {
      self.postParseTokens[key] = value;
    } else {
      self.parseTokens[key] = value;
    }
  });
}

Parser.prototype.convert = function (schema, text) {
  var parseTree = this.parseTree || (getEngine() && getEngine().tree);
  if (typeof parseTree !== "function") {
    throw new Error("the rich editor has no parser: the markdown crate's node tree export is not loaded");
  }
  var parser = new MarkdownParser(
    schema,
    {
      parse: function (markdown) {
        return tokensFromTree(parseTree(markdown));
      },
    },
    this.parseTokens
  );

  Object.assign(parser.tokenHandlers, this.postParseTokens);

  // An inline node left open by unbalanced HTML must not outlive its block.
  // The length guard spares extensions that swap in a single-entry stack.
  Object.keys(parser.tokenHandlers).forEach(function (type) {
    var handler = parser.tokenHandlers[type];
    parser.tokenHandlers[type] = function (state, token, tokens, i) {
      if (token.block) {
        while (state.stack.length > 1 && state.top().type.isInline) {
          state.closeNode();
        }
      }
      return handler(state, token, tokens, i);
    };
  });

  try {
    return parser.parse(text);
  } catch (e) {
    if (e && typeof e.message === "string" && e.message.indexOf("Token type") === 0) {
      throw new UnsupportedTokenError(e.message);
    }
    throw e;
  }
};

Parser.prototype.extractParsers = function (extensions) {
  var parsers = {};
  var self = this;
  extensions.forEach(function (extension) {
    var parseObj = extension.parse;
    if (!parseObj) {
      return;
    }
    if (parseObj instanceof Function) {
      parseObj = parseObj(self.params);
    }
    Object.keys(parseObj).forEach(function (token) {
      var parseSpec = parseObj[token];
      if (parsers[token] !== undefined) {
        if (self.multipleParseSpecs[token] === undefined) {
          self.multipleParseSpecs[token] = [parsers[token]];
          parsers[token] = self.multipleParser(token);
        }
        self.multipleParseSpecs[token].push(parseSpec);
        return;
      }
      parsers[token] = parseSpec;
    });
  });
  return parsers;
};

Parser.prototype.multipleParser = function (tokenName) {
  var self = this;
  return function (state, token, tokens, i) {
    var parseSpecs = self.multipleParseSpecs[tokenName];
    for (var j = 0; j < parseSpecs.length; j++) {
      if (parseSpecs[j](state, token, tokens, i)) {
        return;
      }
    }
    throw new UnsupportedTokenError("no parser handled " + tokenName);
  };
};
