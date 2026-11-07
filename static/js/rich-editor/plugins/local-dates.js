// discourse-local-dates' lib/rich-editor-extension.js and
// lib/local-date-node-view.js: [date=...] and [date-range from=... to=...]
// as date chips (span_open span.discourse-local-date in the node tree),
// drawn as the post draws them (local-dates.js formatLocalDate) with an
// edit button that opens the insert date modal over them
// (getContext().modal), and Tab from a selected chip to its button.
import { buildBBCodeAttrs } from "discourse/rich-editor/plugin-utils";
import { iconHtml } from "discourse/rich-editor/toolbar";

var DATE_NODE_TYPES = ["local_date", "local_date_range"];
var EDIT_BUTTON_CLASS = "composer-local-date__edit-button";

var OPTIONAL_DATA_ATTRS = ["format", "recurring", "timezones", "countdown", "displayedTimezone"];

// lib/format-local-date.js; the composer loads moment before the editor.
function formatLocalDate(date, time, timezone, options) {
  var localDates = window.Discourse && window.Discourse.localDates;
  if (!localDates || !window.moment || !window.moment.tz) {
    return { formatted: [date, time].filter(Boolean).join(" "), pastEvent: false };
  }
  return localDates.formatLocalDate(date, time, timezone, options);
}

function wrapDateRangeSpans(element) {
  var dateSpans = element.querySelectorAll("span.discourse-local-date[data-range]");
  var processed = new Set();

  dateSpans.forEach(function (span) {
    if (processed.has(span) || span.closest(".discourse-local-date-range")) {
      return;
    }

    var range = span.dataset.range;
    if (range !== "true" && range !== "from") {
      return;
    }

    var toSpan = span.nextSibling;
    while (toSpan && toSpan.nodeType === Node.TEXT_NODE) {
      toSpan = toSpan.nextSibling;
    }

    if (!toSpan || !toSpan.classList || !toSpan.classList.contains("discourse-local-date") || !toSpan.dataset.range) {
      return;
    }

    processed.add(span);
    processed.add(toSpan);

    span.dataset.range = "from";
    toSpan.dataset.range = "to";

    var doc = element.nodeType === Node.DOCUMENT_NODE ? element : element.ownerDocument;
    var wrapper = doc.createElement("span");
    wrapper.className = "discourse-local-date-range";
    span.parentNode.insertBefore(wrapper, span);
    wrapper.appendChild(span);

    var node = wrapper.nextSibling;
    while (node && node !== toSpan) {
      var next = node.nextSibling;
      wrapper.appendChild(node);
      node = next;
    }

    wrapper.appendChild(toSpan);
  });
}

function addOptionalDataAttrs(attrs, nodeAttrs, keys) {
  (keys || OPTIONAL_DATA_ATTRS).forEach(function (key) {
    if (nodeAttrs[key]) {
      var dataKey = key === "displayedTimezone" ? "data-displayed-timezone" : "data-" + key;
      attrs[dataKey] = nodeAttrs[key];
    }
  });
}

function buildFormatOptions(nodeAttrs, includeRecurring) {
  var options = {
    format: nodeAttrs.format,
    countdown: nodeAttrs.countdown,
    displayedTimezone: nodeAttrs.displayedTimezone,
    timezones: nodeAttrs.timezones ? nodeAttrs.timezones.split("|") : undefined,
  };
  if (includeRecurring) {
    options.recurring = nodeAttrs.recurring;
  }
  return options;
}

function without(attrs, keys) {
  var rest = {};
  Object.keys(attrs).forEach(function (key) {
    if (keys.indexOf(key) === -1) {
      rest[key] = attrs[key];
    }
  });
  return rest;
}

function nodeToInitialValues(node) {
  var attrs = node.attrs;
  var isRange = node.type.name === "local_date_range";

  return {
    date: isRange ? attrs.fromDate : attrs.date,
    time: isRange ? attrs.fromTime : attrs.time,
    toDate: isRange ? attrs.toDate : null,
    toTime: isRange ? attrs.toTime : null,
    timezone: attrs.timezone,
    format: attrs.format,
    timezones: attrs.timezones ? attrs.timezones.split("|") : [],
    recurring: isRange ? null : attrs.recurring,
    countdown: attrs.countdown,
    displayedTimezone: attrs.displayedTimezone,
  };
}

function LocalDateNodeView(node, view, getPos, params) {
  var self = this;
  this.node = node;
  this.view = view;
  this.getPos = getPos;
  this.getContext = params.getContext;
  this.convertFromMarkdown = params.utils.convertFromMarkdown;
  this.NodeSelection = params.pmState.NodeSelection;
  this.DOMSerializer = params.pmModel.DOMSerializer;

  this.returnFocusToEditor = function (event) {
    if (event.key !== "Tab") {
      return;
    }

    event.preventDefault();
    self.view.focus();
  };

  this.openEditModal = function (event) {
    event.preventDefault();
    event.stopPropagation();

    self.getContext().modal.show("local-dates-create", {
      model: {
        initialValues: nodeToInitialValues(self.node),
        insertDate: function (markup) {
          self.applyEdit(markup);
        },
      },
    });
  };

  this.dom = document.createElement("span");
  this.dom.classList.add("composer-local-date");
  this.dom.setAttribute("contenteditable", "false");

  this.editButton = document.createElement("button");
  this.editButton.type = "button";
  this.editButton.classList.add(EDIT_BUTTON_CLASS);
  this.editButton.innerHTML = iconHtml("pencil");
  this.editButton.addEventListener("click", this.openEditModal);
  this.editButton.addEventListener("keydown", this.returnFocusToEditor);

  this.render();
}

LocalDateNodeView.prototype.update = function (node) {
  var changed = !this.node.sameMarkup(node);
  this.node = node;

  if (changed) {
    this.render();
  }

  return true;
};

LocalDateNodeView.prototype.selectNode = function () {
  this.dom.classList.add("ProseMirror-selectednode");
};

LocalDateNodeView.prototype.deselectNode = function () {
  this.dom.classList.remove("ProseMirror-selectednode");
};

LocalDateNodeView.prototype.stopEvent = function (event) {
  return this.editButton.contains(event.target);
};

LocalDateNodeView.prototype.ignoreMutation = function () {
  return true;
};

LocalDateNodeView.prototype.destroy = function () {
  this.editButton.removeEventListener("click", this.openEditModal);
  this.editButton.removeEventListener("keydown", this.returnFocusToEditor);
};

// The chip is the node spec's toDOM, so it matches what the schema draws.
LocalDateNodeView.prototype.render = function () {
  var dom = this.DOMSerializer.renderSpec(document, this.node.type.spec.toDOM(this.node)).dom;

  this.dom.replaceChildren(dom, this.editButton);

  // Named after its date, so several chips are told apart when read aloud.
  var label = this.getContext().i18n("discourse_local_dates.edit_date", { date: dom.textContent.trim() });
  this.editButton.setAttribute("title", label);
  this.editButton.setAttribute("aria-label", label);
};

LocalDateNodeView.prototype.applyEdit = function (markup) {
  var pos = this.getPos();

  if (pos === undefined) {
    return;
  }

  // The modal hands back one date's bbcode: one paragraph.
  var first = this.convertFromMarkdown(markup).firstChild;
  var content = first && first.content;

  if (!content) {
    return;
  }

  var tr = this.view.state.tr.replaceWith(pos, pos + this.node.nodeSize, content);
  tr.setSelection(this.NodeSelection.create(tr.doc, pos));
  this.view.dispatch(tr);
  this.view.focus();
};

function createLocalDateNodeView(params) {
  return function (node, view, getPos) {
    return new LocalDateNodeView(node, view, getPos, params);
  };
}

// Tab is the composer's, so a selected chip's edit button would be out of
// a keyboard's reach without this.
function focusEditButtonPlugin(params) {
  var Plugin = params.pmState.Plugin;
  var NodeSelection = params.pmState.NodeSelection;

  return new Plugin({
    props: {
      handleKeyDown: function (view, event) {
        if (event.key !== "Tab" || event.shiftKey) {
          return false;
        }

        var selection = view.state.selection;

        if (!(selection instanceof NodeSelection) || DATE_NODE_TYPES.indexOf(selection.node.type.name) === -1) {
          return false;
        }

        var dom = view.nodeDOM(selection.from);
        var button = dom && dom.querySelector ? dom.querySelector("." + EDIT_BUTTON_CLASS) : null;

        if (!button) {
          return false;
        }

        button.focus();

        return true;
      },
    },
  });
}

export default {
  nodeViews: {
    local_date: createLocalDateNodeView,
    local_date_range: createLocalDateNodeView,
  },

  nodeSpec: {
    local_date: {
      attrs: {
        date: {},
        time: { default: null },
        timezone: { default: null },
        format: { default: null },
        recurring: { default: null },
        timezones: { default: null },
        countdown: { default: null },
        displayedTimezone: { default: null },
      },
      group: "inline",
      inline: true,
      parseDOM: [
        {
          tag: "span.discourse-local-date[data-date]",
          getAttrs: function (dom) {
            // a range's spans are local_date_range's, by their wrapper
            if (dom.dataset.range) {
              return false;
            }
            return {
              date: dom.dataset.date,
              time: dom.dataset.time,
              timezone: dom.dataset.timezone,
              format: dom.dataset.format,
              recurring: dom.dataset.recurring,
              timezones: dom.dataset.timezones,
              countdown: dom.dataset.countdown,
              displayedTimezone: dom.dataset.displayedTimezone,
            };
          },
        },
      ],
      toDOM: function (node) {
        var formatted = formatLocalDate(
          node.attrs.date,
          node.attrs.time,
          node.attrs.timezone,
          buildFormatOptions(node.attrs, true)
        ).formatted;
        var attrs = {
          class: "discourse-local-date cooked-date",
          "data-date": node.attrs.date,
          "data-time": node.attrs.time,
          "data-timezone": node.attrs.timezone,
        };
        addOptionalDataAttrs(attrs, node.attrs);
        return ["span", attrs, formatted];
      },
    },
    local_date_range: {
      attrs: {
        fromDate: {},
        toDate: { default: null },
        fromTime: { default: null },
        toTime: { default: null },
        timezone: { default: null },
        format: { default: null },
        timezones: { default: null },
        countdown: { default: null },
        displayedTimezone: { default: null },
      },
      group: "inline",
      inline: true,
      parseDOM: [
        {
          tag: "span.discourse-local-date-range",
          getAttrs: function (dom) {
            var fromSpan = dom.querySelector('[data-range="from"]');
            var toSpan = dom.querySelector('[data-range="to"]');
            if (!fromSpan) {
              return false;
            }
            return {
              fromDate: fromSpan.dataset.date,
              toDate: toSpan ? toSpan.dataset.date : undefined,
              fromTime: fromSpan.dataset.time,
              toTime: toSpan ? toSpan.dataset.time : undefined,
              timezone: fromSpan.dataset.timezone,
              format: fromSpan.dataset.format,
              timezones: fromSpan.dataset.timezones,
              countdown: fromSpan.dataset.countdown,
              displayedTimezone: fromSpan.dataset.displayedTimezone,
            };
          },
        },
      ],
      toDOM: function (node) {
        var options = buildFormatOptions(node.attrs);
        var formattedFrom = formatLocalDate(
          node.attrs.fromDate,
          node.attrs.fromTime,
          node.attrs.timezone,
          options
        ).formatted;
        var formattedTo = formatLocalDate(node.attrs.toDate, node.attrs.toTime, node.attrs.timezone, options).formatted;
        var rangeAttrs = ["format", "timezones", "countdown", "displayedTimezone"];
        var fromAttrs = {
          class: "discourse-local-date cooked-date",
          "data-range": "from",
          "data-date": node.attrs.fromDate,
          "data-time": node.attrs.fromTime,
          "data-timezone": node.attrs.timezone,
        };
        var toAttrs = {
          class: "discourse-local-date cooked-date",
          "data-range": "to",
          "data-date": node.attrs.toDate,
          "data-time": node.attrs.toTime,
          "data-timezone": node.attrs.timezone,
        };
        addOptionalDataAttrs(fromAttrs, node.attrs, rangeAttrs);
        addOptionalDataAttrs(toAttrs, node.attrs, rangeAttrs);
        return [
          "span",
          { class: "discourse-local-date-range" },
          ["span", fromAttrs, formattedFrom],
          " → ",
          ["span", toAttrs, formattedTo],
        ];
      },
    },
  },

  plugins: focusEditButtonPlugin,

  parse: {
    span_open: function (state, token, tokens, i) {
      if (token.attrGet("class") !== "discourse-local-date") {
        return;
      }

      if (token.attrGet("data-range") === "from") {
        state.openNode(state.schema.nodes.local_date_range, {
          fromDate: token.attrGet("data-date"),
          fromTime: token.attrGet("data-time"),
          timezone: token.attrGet("data-timezone"),
          format: token.attrGet("data-format"),
          timezones: token.attrGet("data-timezones"),
          countdown: token.attrGet("data-countdown"),
          displayedTimezone: token.attrGet("data-displayed-timezone"),
        });
        state.__localDateRange = true;
        // The range's tokens are strictly [span_open, text, span_close,
        // text, span_open, text, span_close]: the texts go.
        tokens.splice(i + 1, 1);
        tokens.splice(i + 2, 1);
        tokens.splice(i + 3, 1);

        return true;
      }

      if (token.attrGet("data-range") === "to") {
        // No node for `to`: its date goes on the range.
        state.top().attrs.toDate = token.attrGet("data-date");
        state.top().attrs.toTime = token.attrGet("data-time");
        delete state.__localDateRange;
        return true;
      }

      state.openNode(state.schema.nodes.local_date, {
        date: token.attrGet("data-date"),
        time: token.attrGet("data-time"),
        timezone: token.attrGet("data-timezone"),
        format: token.attrGet("data-format"),
        recurring: token.attrGet("data-recurring"),
        timezones: token.attrGet("data-timezones"),
        countdown: token.attrGet("data-countdown"),
        displayedTimezone: token.attrGet("data-displayed-timezone"),
      });
      // its text goes
      tokens.splice(i + 1, 1);
      return true;
    },
    span_close: function (state) {
      if (DATE_NODE_TYPES.indexOf(state.top().type.name) !== -1) {
        if (!state.__localDateRange) {
          state.closeNode();
        }
        return true;
      }
    },
  },
  serializeNode: function (params) {
    var isBoundary = params.utils.isBoundary;
    return {
      local_date: function (state, node, parent, index) {
        state.flushClose();
        if (!isBoundary(state.out, state.out.length - 1)) {
          state.write(" ");
        }

        var optionalAttrs = buildBBCodeAttrs(without(node.attrs, ["date"]));
        state.write("[date=" + node.attrs.date + (optionalAttrs ? " " + optionalAttrs : "") + "]");

        var nextSibling = parent.childCount > index + 1 ? parent.child(index + 1) : null;
        if (nextSibling && nextSibling.isText && !isBoundary(nextSibling.text, 0)) {
          state.write(" ");
        }
      },
      local_date_range: function (state, node, parent, index) {
        state.flushClose();
        if (!isBoundary(state.out, state.out.length - 1)) {
          state.write(" ");
        }

        var attrs = node.attrs;
        var from = attrs.fromDate + (attrs.fromTime ? "T" + attrs.fromTime : "");
        var to = attrs.toDate + (attrs.toTime ? "T" + attrs.toTime : "");
        var optionalAttrs = buildBBCodeAttrs(without(attrs, ["fromDate", "toDate", "fromTime", "toTime"]));
        state.write("[date-range from=" + from + " to=" + to + (optionalAttrs ? " " + optionalAttrs : "") + "]");

        var nextSibling = parent.childCount > index + 1 ? parent.child(index + 1) : null;
        if (nextSibling && nextSibling.isText && !isBoundary(nextSibling.text, 0)) {
          state.write(" ");
        }
      },
    };
  },

  // Adjacent date-range spans wrapped before the HTML is parsed.
  transformParsedHTML: function (element) {
    wrapDateRangeSpans(element);
  },
};
