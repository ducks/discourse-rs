// The rich editor's own small toolbars (the link toolbar, the image
// toolbar): lib/composer/toolbar.ts's ToolbarBase, which holds the buttons,
// and components/composer/toolbar-buttons.gjs, drawn here as plain DOM
// (renderToolbarButtons) with DButton's markup, lib/roving-button-bar.js
// for the arrow keys, and the icon markup of lib/icon-library's iconHTML.
// Titles and labels are i18n keys, looked up through the editor context's
// i18n (editor.js).

var DEFAULT_GROUP = "main";

function escapeHtml(s) {
  return String(s === undefined || s === null ? "" : s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#x27;");
}

// iconHTML
export function iconHtml(name, extraClass) {
  return (
    '<svg class="fa d-icon d-icon-' +
    escapeHtml(name) +
    " svg-icon fa-width-auto" +
    (extraClass ? " " + escapeHtml(extraClass) : "") +
    ' svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
    escapeHtml(name) +
    '"></use></svg>'
  );
}

// iconElement
export function iconElement(name, extraClass) {
  var template = document.createElement("template");
  template.innerHTML = iconHtml(name, extraClass);
  return template.content.firstChild;
}

export function ToolbarBase(opts) {
  opts = opts || {};
  this.groups = [{ group: DEFAULT_GROUP, buttons: [] }];
  this.i18n =
    opts.i18n ||
    function (key) {
      return key;
    };
}

// The button keeps the attrs' getters (an icon or a disabled state that
// follows the editor), as Object.defineProperties does in the reference.
ToolbarBase.prototype.addButton = function (buttonAttrs) {
  var group = this.groups.find(function (item) {
    return item.group === (buttonAttrs.group || DEFAULT_GROUP);
  });
  var button = Object.defineProperties({}, Object.getOwnPropertyDescriptors(buttonAttrs));

  if (button.preventFocus === undefined) {
    button.preventFocus = true;
  }
  if (button.tabindex === undefined) {
    button.tabindex = "-1";
  }
  if (!button.className) {
    button.className = buttonAttrs.id;
  }
  if (!button.condition) {
    button.condition = function () {
      return true;
    };
  }
  button.action = function () {
    var actionFn = buttonAttrs.action || buttonAttrs.sendAction || buttonAttrs.perform;
    if (actionFn) {
      actionFn();
    }
  };
  button.title = this.i18n(buttonAttrs.title || "composer." + buttonAttrs.id + "_title");

  if (buttonAttrs.unshift) {
    group.buttons.unshift(button);
  } else {
    group.buttons.push(button);
  }
};

ToolbarBase.prototype.addSeparator = function (opts) {
  var groupName = (opts && opts.group) || DEFAULT_GROUP;
  var group = this.groups.find(function (item) {
    return item.group === groupName;
  });
  if (!group) {
    throw new Error("Couldn't find toolbar group " + groupName);
  }
  group.buttons.push({
    type: "separator",
    condition: (opts && opts.condition) || function () {
      return true;
    },
  });
};

function isActionable(element) {
  if (!element) {
    return false;
  }
  return (
    element.disabled !== true &&
    (element.tagName === "BUTTON" ||
      element.tagName === "A" ||
      element.getAttribute("role") === "switch" ||
      element.getAttribute("role") === "button") &&
    !element.classList.contains("select-kit") &&
    !element.classList.contains("hidden")
  );
}

// lib/roving-button-bar.js
export function rovingButtonBar(event, containerClass) {
  var target = event.target;
  var siblingFinder;

  if (event.code === "ArrowRight") {
    siblingFinder = "nextElementSibling";
  } else if (event.code === "ArrowLeft") {
    siblingFinder = "previousElementSibling";
  } else {
    return false;
  }

  if (containerClass) {
    while (target.parentNode && !target.parentNode.classList.contains(containerClass)) {
      target = target.parentNode;
    }
  }

  var focusable = target[siblingFinder];
  while (focusable && !isActionable(focusable)) {
    focusable = focusable[siblingFinder];
    if (focusable && focusable.tagName === "DETAILS") {
      focusable = focusable.querySelector("summary");
    }
  }

  if (!focusable) {
    return false;
  }

  focusable.focus();
  return true;
}

// DButton with an icon, a label or both, or as a link with an href.
function buttonElement(button, tabindex, i18n, onKeyDown) {
  var label = button.translatedLabel || (button.label ? i18n(button.label) : "");
  var el = document.createElement(button.href ? "a" : "button");
  var classes = ["btn"];
  if (button.icon && label) {
    classes.push("btn-icon-text");
  } else if (button.icon) {
    classes.push("no-text", "btn-icon");
  } else {
    classes.push("btn-text");
  }
  classes.push("toolbar__button");
  if (button.className) {
    classes.push(button.className);
  }
  el.className = classes.join(" ");
  if (button.href) {
    el.href = button.href;
    el.rel = "noopener noreferrer";
    el.target = "_blank";
  } else {
    el.type = "button";
    if (button.disabled) {
      el.disabled = true;
    }
  }
  el.setAttribute("tabindex", tabindex);
  if (button.title) {
    el.title = button.title;
  }
  var html = "";
  if (button.icon) {
    html += iconHtml(button.icon);
  }
  if (label) {
    html += '<span class="d-button-label">' + escapeHtml(label) + "</span>";
  } else if (button.icon) {
    html += '<span aria-hidden="true">&#8203;</span>';
  }
  el.innerHTML = html;

  if (button.preventFocus) {
    el.addEventListener("mousedown", function (event) {
      event.preventDefault();
    });
  }
  if (!button.href) {
    el.addEventListener("click", function (event) {
      event.preventDefault();
      button.action();
    });
  }
  el.addEventListener("keydown", function (event) {
    onKeyDown(event);
  });
  return el;
}

// ToolbarButtons into `container` (emptied first), the first visible
// actionable button the tab stop. Call again to redraw after the state the
// buttons read changes.
export function renderToolbarButtons(container, toolbar, opts) {
  opts = opts || {};
  var i18n = toolbar.i18n;
  var roving = toolbar.rovingButtonBar || rovingButtonBar;
  var visible = function (button) {
    return !button.hiddenByUser && button.condition();
  };
  var first = null;
  toolbar.groups.forEach(function (group) {
    group.buttons.forEach(function (button) {
      if (!first && button.type !== "separator" && !button.disabled && visible(button)) {
        first = button;
      }
    });
  });

  // A redraw keeps the focused button focused, as Glimmer keeps its
  // element.
  var focused = container.contains(document.activeElement)
    ? Array.prototype.indexOf.call(container.children, document.activeElement)
    : -1;
  container.replaceChildren();
  toolbar.groups.forEach(function (group) {
    group.buttons.forEach(function (button) {
      if (!visible(button)) {
        return;
      }
      if (button.type === "separator") {
        var separator = document.createElement("div");
        separator.className = "toolbar-separator";
        container.appendChild(separator);
        return;
      }
      var tabindex = opts.isFirst !== false && button === first ? "0" : button.tabindex;
      container.appendChild(buttonElement(button, tabindex, i18n, roving));
    });
  });
  if (focused >= 0 && container.children[focused]) {
    container.children[focused].focus();
  }
  return container;
}

// toasts.success({ duration: "short", data: { message } }), as
// static/js/quote-button.js draws it.
export function toastSuccess(text) {
  var section = document.querySelector("section.fk-d-toasts");
  if (!section) {
    section = document.createElement("section");
    section.className = "fk-d-toasts";
    document.body.appendChild(section);
  }
  var output = document.createElement("output");
  output.className = "fk-d-toast";
  output.setAttribute("role", "status");
  output.innerHTML =
    '<div class="fk-d-default-toast -success"><div class="fk-d-default-toast__icon-container">' +
    iconHtml("check") +
    '</div><div class="fk-d-default-toast__main-container"><div class="fk-d-default-toast__texts"><div class="fk-d-default-toast__message"></div></div></div><div class="fk-d-default-toast__close-container"><button class="btn no-text btn-icon btn-transparent" type="button">' +
    iconHtml("xmark") +
    '<span aria-hidden="true">&#8203;</span></button></div></div>';
  output.querySelector(".fk-d-default-toast__message").textContent = text;
  section.appendChild(output);
  var close = function () {
    output.remove();
  };
  output.querySelector("button").addEventListener("click", close);
  setTimeout(close, 3000);
}
