// discourse-poll's components/poll-node-view.gjs in GlimmerNodeView's
// shell (lib/glimmer-node-view.js), with components/poll-info.gjs: the
// poll's content (title and options, edited in the document), then what
// the poll will say (PollInfo with no voters yet) and Edit poll, which
// opens the builder over its settings (getContext().modal) and writes
// them back, a number poll's options generated again from its range.
import { buildBBCodeAttrs } from "discourse/rich-editor/plugin-utils";
import { iconHtml } from "discourse/rich-editor/toolbar";

var ON_VOTE = "on_vote";
var ON_CLOSE = "on_close";
var STAFF_ONLY = "staff_only";

function escapeHtml(s) {
  return String(s === null || s === undefined ? "" : s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

// lib/poll-bounds.js: no minimum is one, no maximum (or one past the
// options there are) all of them.
export function pollBounds(poll, optionCount) {
  var min = parseInt(poll.min, 10);
  var max = parseInt(poll.max, 10);

  return {
    min: isNaN(min) || min < 0 ? 1 : min,
    max: isNaN(max) || max > optionCount ? optionCount : max,
  };
}

// moment(close): a date alone is local midnight.
function parseClose(close) {
  var date = /^(\d{4})-(\d{2})-(\d{2})$/.exec(close);
  var ms = date ? new Date(+date[1], +date[2] - 1, +date[3]).getTime() : Date.parse(close);
  return isNaN(ms) ? null : ms;
}

function dates() {
  return (window.Discourse && window.Discourse.dates) || null;
}

// components/poll-info.gjs, as the node view renders it: no results shown,
// nothing voted, not closed by anyone, the poll not yet posted.
export function pollInfoHtml(info, i18n, currentUser) {
  var optionsCount = info.options.length;
  var min = info.min;
  var max = info.max;

  function multipleHelpText() {
    if (max > 0) {
      if (min === max && min > 1) {
        return i18n("poll.multiple.help.x_options", { count: min });
      }
      if (min > 1) {
        if (max < optionsCount) {
          return i18n("poll.multiple.help.between_min_and_max_options", { min: min, max: max });
        }
        return i18n("poll.multiple.help.at_least_min_options", { count: min });
      }
      if (max <= optionsCount) {
        return i18n("poll.multiple.help.up_to_max_options", { count: max });
      }
    }
    return "";
  }

  var closed = false;
  var showMultipleHelpText = info.isMultiple && !closed;
  var showAutomaticClose = info.isAutomaticallyClosed || (info.closesAt !== null && !closed);
  // the post's author is not known: the current user's id (none here)
  // stands for it, as an unsaved post's
  var postUserId = currentUser ? currentUser.id : undefined;
  var resultsOnVote = info.results === ON_VOTE && !(currentUser && postUserId === currentUser.id);
  var resultsOnClose = info.results === ON_CLOSE && !closed;
  var resultsStaffOnly = info.results === STAFF_ONLY && !(currentUser && currentUser.staff);
  var publicTitle = !closed && info.isPublic && info.results !== STAFF_ONLY;
  var showInstructions =
    showMultipleHelpText ||
    showAutomaticClose ||
    resultsOnVote ||
    resultsOnClose ||
    resultsStaffOnly ||
    publicTitle ||
    info.isDynamic;

  function li(className, icon, html, title) {
    return (
      "<li" +
      (className ? ' class="' + className + '"' : "") +
      (title !== undefined ? ' title="' + escapeHtml(title) + '"' : "") +
      ">" +
      iconHtml(icon) +
      "<span>" +
      html +
      "</span></li>"
    );
  }

  var html =
    '<div class="poll-info"><div class="poll-info_counts"><div class="poll-info_counts-count"><span class="info-number">' +
    info.voters +
    '</span> <span class="info-label">' +
    escapeHtml(i18n("poll.voters", { count: info.voters })) +
    "</span></div></div>";

  if (showInstructions) {
    html += '<ul class="poll-info_instructions">';
    if (info.isDynamic) {
      html += li("is-dynamic", "shuffle", escapeHtml(i18n("poll.dynamic.enabled_hint")));
    }
    if (showMultipleHelpText) {
      html += li("multiple-help-text", "list-ul", multipleHelpText());
    }
    if (showAutomaticClose) {
      var d = dates();
      var label = info.isAutomaticallyClosed
        ? i18n("poll.automatic_close.age", { age: d ? d.relativeAgeWithAgo(info.closesAt) : "" })
        : i18n("poll.automatic_close.closes_in", { timeLeft: d ? d.humanize(info.closesAt) : "" });
      html += li(null, info.isAutomaticallyClosed ? "lock" : "far-clock", label, d ? d.format(info.closesAt, "LLL") : "");
    }
    if (resultsOnVote) {
      html += li("results-on-vote", "check", i18n("poll.results.vote.title"));
    }
    if (resultsOnClose) {
      html += li("results-on-close", "lock", i18n("poll.results.closed.title"));
    }
    if (resultsStaffOnly) {
      html += li("results-staff-only", "shield-halved", i18n("poll.results.staff.title"));
    }
    if (publicTitle) {
      html += li("is-public", "far-eye", i18n("poll.public.title"));
    }
    html += "</ul>";
  }

  return html + "</div>";
}

export function PollNodeView(node, view, getPos, params) {
  var self = this;
  this.node = node;
  this.view = view;
  this.getPos = getPos;
  this.params = params;
  var i18n = params.getContext().i18n;

  this.dom = document.createElement("div");
  this.dom.classList.add("composer-poll-node");
  this.contentDOM = document.createElement("div");
  this.contentDOM.classList.add("poll-container", "composer-poll-node__content");
  this.dom.appendChild(this.contentDOM);
  this.dom.style.setProperty("--poll-title-placeholder", JSON.stringify(i18n("poll.ui_builder.poll_title.placeholder")));

  this.info = document.createElement("div");
  this.info.className = "composer-poll-node__info";
  this.info.contentEditable = "false";
  this.dom.appendChild(this.info);

  var actions = document.createElement("div");
  actions.className = "poll-buttons composer-poll-node__actions";
  actions.contentEditable = "false";
  this.editButton = document.createElement("button");
  this.editButton.className = "btn btn-icon-text btn-default composer-poll-node__edit";
  this.editButton.type = "button";
  this.editButton.innerHTML =
    iconHtml("pencil") + '<span class="d-button-label">' + escapeHtml(i18n("poll.ui_builder.edit")) + "</span>";
  // @preventFocus
  this.editButton.addEventListener("mousedown", function (event) {
    event.preventDefault();
  });
  this.editButton.addEventListener("click", function () {
    self.editPoll();
  });
  actions.appendChild(this.editButton);
  this.dom.appendChild(actions);

  this.update(node);
}

PollNodeView.prototype.optionList = function (node) {
  var bulletList = this.params.schema.nodes.bullet_list;
  var offset = 1;

  for (var index = 0; index < node.childCount; index++) {
    var child = node.child(index);
    if (child.type === bulletList) {
      return { node: child, offset: offset };
    }
    offset += child.nodeSize;
  }
  return null;
};

PollNodeView.prototype.optionCount = function () {
  var list = this.optionList(this.node);
  return list ? list.node.childCount : 0;
};

PollNodeView.prototype.pollInfo = function () {
  var attrs = this.node.attrs;
  var closesAt = attrs.close ? parseClose(attrs.close) : null;
  var bounds = pollBounds(attrs, this.optionCount());

  return {
    isMultiple: attrs.type === "multiple",
    isPublic: attrs.public === "true",
    isDynamic: attrs.dynamic === "true",
    results: attrs.results,
    min: bounds.min,
    max: bounds.max,
    closesAt: closesAt,
    isAutomaticallyClosed: closesAt !== null && closesAt < Date.now(),
    // only the count describes the selection limits
    options: new Array(this.optionCount()),
    voters: 0,
  };
};

PollNodeView.prototype.drawInfo = function () {
  var context = this.params.getContext();
  this.info.innerHTML = pollInfoHtml(this.pollInfo(), context.i18n, context.currentUser);
};

PollNodeView.prototype.editPoll = function () {
  var self = this;
  var node = this.node;

  this.params.getContext().modal.show("poll-ui-builder", {
    model: {
      poll: Object.assign({}, node.attrs, { optionCount: this.optionCount() }),
      onSave: function (attrs) {
        self.savePoll(attrs);
      },
    },
  });
};

PollNodeView.prototype.ignoreMutation = function (mutation) {
  return (
    mutation.type !== "selection" &&
    ((mutation.type === "attributes" && mutation.target === this.contentDOM) ||
      !this.contentDOM.contains(mutation.target))
  );
};

PollNodeView.prototype.selectNode = function () {
  this.dom.classList.add("ProseMirror-selectednode");
};

PollNodeView.prototype.deselectNode = function () {
  this.dom.classList.remove("ProseMirror-selectednode");
};

PollNodeView.prototype.stopEvent = function (event) {
  return !!(event.target.closest && event.target.closest(".composer-poll-node__edit"));
};

PollNodeView.prototype.update = function (node) {
  this.node = node;
  var attrs = node.type.spec.toDOM(node)[1];
  var dom = this.dom;
  Object.keys(attrs).forEach(function (name) {
    var value = attrs[name];
    if (name === "class") {
      dom.classList.add(value);
    } else if (value === null) {
      dom.removeAttribute(name);
    } else {
      dom.setAttribute(name, value);
    }
  });
  // An empty editor block holds a trailing break, so :empty cannot tell.
  dom.classList.toggle(
    "--untitled",
    !!node.firstChild && node.firstChild.type.name === "poll_title" && node.firstChild.content.size === 0
  );

  // A number poll's options are generated, and left out of the markdown.
  var list = this.contentDOM.querySelector(":scope > ul");
  if (list) {
    if (node.attrs.type === "number") {
      list.contentEditable = "false";
    } else {
      list.removeAttribute("contenteditable");
    }
  }

  this.drawInfo();
  return true;
};

PollNodeView.prototype.savePoll = function (attrs) {
  var view = this.view;
  var params = this.params;
  var pos = this.getPos();
  if (pos === undefined || view.isDestroyed) {
    return;
  }

  var node = view.state.doc.nodeAt(pos);
  if (!node || node.type !== this.node.type) {
    return;
  }

  var tr = view.state.tr.setNodeMarkup(pos, null, attrs);

  if (attrs.type === "number") {
    var generatedPoll = params.utils.convertFromMarkdown("[poll " + buildBBCodeAttrs(attrs) + "]\n[/poll]").firstChild;
    var generatedList = generatedPoll && this.optionList(generatedPoll);
    var generated = generatedList && generatedList.node;
    var list = this.optionList(node);

    if (list && generated && generated.type === list.node.type) {
      var from = pos + list.offset;
      tr.replaceWith(from, from + list.node.nodeSize, generated);
    }
  }

  tr.setSelection(params.pmState.NodeSelection.create(tr.doc, pos));
  view.dispatch(tr);
  view.focus();
};
