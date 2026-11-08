// discourse-policy's initializers/rich-editor-extension.js: [policy ...] as
// a policy node (wrap_bbcode div.policy in the node tree) around its
// content, drawn with a header, its settings listed and an edit button
// that opens the policy builder over them (getContext().modal).
import { iconHtml } from "discourse/rich-editor/toolbar";
import { camelCaseToDash } from "discourse/rich-editor/wrap-utils";

var POLICY_ATTRIBUTES = {
  group: { default: null },
  groups: { default: null },
  version: { default: "1" },
  accept: { default: null },
  revoke: { default: null },
  reminder: { default: null },
  renew: { default: null },
  renewStart: { default: null },
  addUsersToGroup: { default: null },
  private: { default: null },
};

var DISPLAYED_POLICY_ATTRIBUTES = [
  "groups",
  "group",
  "version",
  "renew",
  "renewStart",
  "reminder",
  "accept",
  "revoke",
  "addUsersToGroup",
  "private",
];

// @ember/string's camelize
function camelize(str) {
  return str
    .replace(/(-|_|\.|\s)+(.)?/g, function (match, separator, chr) {
      return chr ? chr.toUpperCase() : "";
    })
    .replace(/(^|\/)([A-Z])/g, function (match) {
      return match.toLowerCase();
    });
}

function hasAttributeValue(value) {
  return value !== null && value !== undefined && String(value).trim() !== "" && String(value) !== '""';
}

function policyAttrsToFormPolicy(attrs) {
  var policy = {};

  Object.keys(attrs).forEach(function (key) {
    var value = attrs[key];
    if (!hasAttributeValue(value)) {
      return;
    }

    policy[key] = key === "private" ? value === "true" : value;
  });

  if (!policy.groups && policy.group) {
    policy.groups = policy.group;
    delete policy.group;
  }

  return policy;
}

function formPolicyToPolicyAttrs(policy) {
  var attrs = {};

  Object.keys(POLICY_ATTRIBUTES).forEach(function (key) {
    attrs[key] = null;
  });

  Object.keys(policy).forEach(function (key) {
    var value = policy[key];
    if (key === "private") {
      attrs.private = value === true || value === "true" ? "true" : null;
      return;
    }

    attrs[camelize(key)] = hasAttributeValue(value) ? String(value) : null;
  });

  return attrs;
}

function PolicyNodeView(node, view, getPos, params) {
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

    self.getContext().modal.show("policy-builder", {
      model: {
        policy: policyAttrsToFormPolicy(self.node.attrs),
        onApply: function (policy) {
          self.applyEdit(policy);
        },
      },
    });
  };

  this.dom = document.createElement("div");
  this.dom.classList.add("policy");

  this.syncAttributes();

  var header = document.createElement("div");
  header.classList.add("policy-header");
  header.setAttribute("contenteditable", "false");

  var title = document.createElement("span");
  title.textContent = i18n("discourse_policy.title");
  header.appendChild(title);

  this.editButton = document.createElement("button");
  this.editButton.type = "button";
  this.editButton.classList.add("btn-flat", "policy-node-edit-button");
  this.editButton.setAttribute("contenteditable", "false");
  this.editButton.setAttribute("title", i18n("discourse_policy.builder.edit"));
  this.editButton.setAttribute("aria-label", i18n("discourse_policy.builder.edit"));
  this.editButton.innerHTML = iconHtml("pencil");
  this.editButton.addEventListener("click", this.openEditModal);
  header.appendChild(this.editButton);

  this.dom.appendChild(header);

  this.details = document.createElement("dl");
  this.details.classList.add("policy-attrs");
  this.details.setAttribute("contenteditable", "false");
  this.dom.appendChild(this.details);

  this.contentDOM = document.createElement("div");
  this.contentDOM.classList.add("policy-body");
  this.dom.appendChild(this.contentDOM);

  this.syncDetails();
}

PolicyNodeView.prototype.update = function (node) {
  if (node.type !== this.node.type) {
    return false;
  }

  this.node = node;
  this.syncAttributes();
  this.syncDetails();

  return true;
};

PolicyNodeView.prototype.stopEvent = function (event) {
  return event.target instanceof Node ? this.editButton.contains(event.target) : false;
};

PolicyNodeView.prototype.ignoreMutation = function (mutation) {
  return !this.contentDOM.contains(mutation.target);
};

PolicyNodeView.prototype.destroy = function () {
  this.editButton.removeEventListener("click", this.openEditModal);
};

PolicyNodeView.prototype.syncAttributes = function () {
  var self = this;
  Object.keys(POLICY_ATTRIBUTES).forEach(function (key) {
    self.dom.removeAttribute("data-" + camelCaseToDash(key));
  });

  Object.keys(this.node.attrs).forEach(function (key) {
    var value = self.node.attrs[key];
    if (hasAttributeValue(value)) {
      self.dom.setAttribute("data-" + camelCaseToDash(key), value);
    }
  });
};

PolicyNodeView.prototype.syncDetails = function () {
  var self = this;
  var i18n = this.getContext().i18n;
  this.details.replaceChildren();

  DISPLAYED_POLICY_ATTRIBUTES.forEach(function (key) {
    var value = self.node.attrs[key];

    if (!hasAttributeValue(value)) {
      return;
    }

    var name = document.createElement("dt");
    var labelKey = key === "group" ? "groups" : camelCaseToDash(key);
    name.textContent = i18n("discourse_policy.builder." + labelKey + ".label");

    var definition = document.createElement("dd");
    definition.textContent = value;

    self.details.append(name, definition);
  });
};

PolicyNodeView.prototype.applyEdit = function (policy) {
  var pos = this.getPos();
  var tr = this.view.state.tr.setNodeMarkup(pos, null, formPolicyToPolicyAttrs(policy));
  tr.setSelection(this.NodeSelection.create(tr.doc, pos));
  this.view.dispatch(tr);
};

function createPolicyNodeView(params) {
  return function (node, view, getPos) {
    return new PolicyNodeView(node, view, getPos, params);
  };
}

export default {
  nodeSpec: {
    policy: {
      attrs: POLICY_ATTRIBUTES,
      group: "block",
      content: "block*",
      createGapCursor: true,
      parseDOM: [
        {
          tag: "div.policy",
          getAttrs: function (dom) {
            var attrs = {};
            Object.keys(dom.dataset).forEach(function (key) {
              var value = dom.dataset[key];
              if (hasAttributeValue(value)) {
                attrs[camelize(key)] = value;
              }
            });
            return attrs;
          },
        },
      ],
      toDOM: function (node) {
        var attrs = { class: "policy" };

        Object.keys(node.attrs).forEach(function (key) {
          var value = node.attrs[key];
          if (hasAttributeValue(value)) {
            attrs["data-" + camelCaseToDash(key)] = value;
          }
        });

        return ["div", attrs, 0];
      },
    },
  },

  nodeViews: {
    policy: createPolicyNodeView,
  },

  parse: {
    wrap_bbcode: function (state, token) {
      if (token.tag === "div") {
        if (token.nesting === -1 && state.top().type.name === "policy") {
          state.closeNode();
          return true;
        }

        if (token.nesting === 1 && token.attrGet("class") === "policy") {
          var attrs = {};
          (token.attrs || []).forEach(function (pair) {
            if (pair[0].startsWith("data-") && hasAttributeValue(pair[1])) {
              attrs[camelize(pair[0].slice(5))] = pair[1];
            }
          });

          state.openNode(state.schema.nodes.policy, attrs);
          return true;
        }
      }

      return false;
    },
  },

  serializeNode: {
    policy: function (state, node) {
      state.write("[policy");

      Object.keys(node.attrs).forEach(function (key) {
        var value = node.attrs[key];
        if (hasAttributeValue(value)) {
          state.write(" " + camelCaseToDash(key) + '="' + value + '"');
        }
      });

      state.write("]\n");

      if (node.content.size > 0) {
        state.renderContent(node);
      }

      state.write("[/policy]\n\n");
    },
  },
};
