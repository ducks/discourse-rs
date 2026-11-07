// components/image-alt-text-input.gjs: the selected image's description,
// shown as text and edited in a textarea once focused or clicked. Blur
// saves it; Enter closes, Escape restores what it was and closes.
export function ImageAltTextInput(container, data) {
  this.data = data;
  this.isExpanded = false;
  this.initialAltText = data.alt || "";
  this.transientAltText = this.initialAltText;
  this.el = document.createElement("div");
  container.appendChild(this.el);
  this.render();
}

ImageAltTextInput.prototype.render = function () {
  var self = this;
  var i18n = this.data.i18n;
  this.el.className = "image-alt-text-input" + (this.isExpanded ? " --expanded" : "");
  this.el.replaceChildren();

  if (this.isExpanded) {
    var textarea = document.createElement("textarea");
    textarea.className = "image-alt-text-input__field";
    textarea.placeholder = i18n("composer.image_alt_text.title");
    textarea.value = this.transientAltText;
    textarea.addEventListener("input", function (event) {
      self.transientAltText = event.target.value;
    });
    textarea.addEventListener("blur", function (event) {
      self.onBlur(event);
    });
    textarea.addEventListener("keydown", function (event) {
      self.onKeyDown(event);
    });
    this.el.appendChild(textarea);
    this.textarea = textarea;
    setTimeout(function () {
      textarea.select();
    }, 0);
  } else {
    var display = document.createElement("div");
    display.className = "image-alt-text-input__display";
    display.tabIndex = 0;
    display.textContent = this.transientAltText || i18n("composer.image_alt_text.title");
    display.addEventListener("focus", function () {
      self.expandInput();
    });
    display.addEventListener("click", function () {
      self.expandInput();
    });
    this.el.appendChild(display);
    this.textarea = null;
  }
};

ImageAltTextInput.prototype.expandInput = function () {
  if (this.isExpanded) {
    return;
  }
  this.isExpanded = true;
  this.render();
  this.textarea.focus();
};

ImageAltTextInput.prototype.onBlur = function (event) {
  // helps avoid a shift of the composer window on mobile
  var forceFocus = event.relatedTarget === this.data.view.dom;
  this.isExpanded = false;
  this.initialAltText = this.transientAltText;
  if (this.data.onSave) {
    this.data.onSave(this.transientAltText.trim(), forceFocus);
  }
};

ImageAltTextInput.prototype.onKeyDown = function (event) {
  event.stopPropagation();
  if (event.key === "Enter") {
    event.preventDefault();
    if (this.data.onClose) {
      this.data.onClose();
    }
  } else if (event.key === "Escape") {
    event.preventDefault();
    this.transientAltText = this.initialAltText;
    if (this.data.onClose) {
      this.data.onClose();
    }
  }
};
