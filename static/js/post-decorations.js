// Two of Ember's post decorators (instance-initializers/post-decorations.js):
//
//   image grid: each div.d-image-grid not in carousel mode is laid out in
//       columns by lib/columns.js. The carousel (lib/image-grid-carousel.js)
//       and the lightbox aren't ported.
//   table-wrapper (onlyStream): each .md-table of a post gets the buttons
//       row, and when it overflows, the button that shows the table in a
//       FullscreenTableModal. The pencil that opens the spreadsheet editor
//       (`--editable`, for a post the viewer can edit) isn't ported.
//
// The pages are the desktop view (site.mobileView false).
(function () {
  "use strict";

  var Discourse = window.Discourse;
  if (!Discourse || !Discourse.decorateCooked || Discourse.postDecorations) {
    return;
  }
  Discourse.postDecorations = true;

  // lib/columns.js: turns an element's children into a grid of columns.
  function Columns(container, options) {
    this.container = container;
    this.options = { columns: 3, minCount: 2 };
    Object.keys(options || {}).forEach(function (key) {
      this.options[key] = options[key];
    }, this);

    this.items = this.prepareItems();

    if (this.items.length >= this.options.minCount) {
      this.render();
    } else {
      container.dataset.disabled = true;
    }
  }

  // A 2x2 grid looks better for 2 or 4 items.
  Columns.prototype.count = function () {
    return [2, 4].indexOf(this.items.length) !== -1 ? 2 : this.options.columns;
  };

  Columns.prototype.render = function () {
    if (this.container.dataset.columns) {
      return;
    }
    this.container.dataset.columns = this.count();
    var columns = this.distributeEvenly();
    this.container.replaceChildren.apply(this.container, columns);
  };

  Columns.prototype.prepareColumns = function (count) {
    var columns = [];
    for (var i = 0; i < count; i++) {
      var column = document.createElement("div");
      column.classList.add("d-image-grid-column");
      columns.push(column);
    }
    return columns;
  };

  // Children wrapped in a paragraph count as the paragraph's children;
  // line breaks and empty paragraphs don't count.
  Columns.prototype.prepareItems = function () {
    var targets = [];
    Array.prototype.slice.call(this.container.children).forEach(function (child) {
      if (child.nodeName === "P" && child.children.length > 0) {
        Array.prototype.slice.call(child.children).forEach(function (c) {
          if (["BR", "P"].indexOf(c.nodeName) === -1) {
            targets.push(c);
          }
        });
      } else if (["BR", "P"].indexOf(child.nodeName) === -1) {
        targets.push(child);
      }
    });
    return targets;
  };

  Columns.prototype.wrapDirectImage = function (item) {
    if (item.nodeName !== "IMG") {
      return item;
    }
    var wrapper = document.createElement("span");
    wrapper.classList.add("image-wrapper");
    wrapper.appendChild(item);
    return wrapper;
  };

  // Each item goes to the column shortest so far, by the images' height to
  // width ratio (1 for an item without an image).
  Columns.prototype.distributeEvenly = function () {
    var count = this.count();
    var columns = this.prepareColumns(count);
    var heights = [];
    for (var i = 0; i < count; i++) {
      heights.push(0);
    }

    this.items.forEach(function (item, index) {
      var shortest = 0;
      for (var j = 1; j < count; ++j) {
        if (heights[j] < heights[shortest]) {
          shortest = j;
        }
      }

      var img = item.querySelector("img") || (item.nodeName === "IMG" ? item : null);
      heights[shortest] += img && img.width > 0 ? img.height / img.width : 1;

      var wrappedItem = this.wrapDirectImage(item);
      var lightboxEl = wrappedItem.matches(".lightbox")
        ? wrappedItem
        : wrappedItem.querySelector(".lightbox");
      if (lightboxEl) {
        lightboxEl.dataset.lightboxPosition = index;
      }
      columns[shortest].append(wrappedItem);
    }, this);

    return columns;
  };

  Discourse.decorateCooked("image-grid", function (el) {
    el.querySelectorAll(".d-image-grid").forEach(function (grid) {
      if (grid.dataset.mode === "carousel") {
        return;
      }
      new Columns(grid, { columns: 3 });
    });
  });

  // _createButton, without a label (labels show on touch devices only).
  function createButton(props) {
    var button = document.createElement("button");
    button.classList.add("open-popup-link", "btn-default", "btn", "btn-icon", "no-text");
    props.classes.forEach(function (c) {
      button.classList.add(c);
    });
    button.title = Discourse.cooked.t(props.title);
    // iconElement: an svg-node icon.
    var svg = Discourse.cooked.icon(props.icon.name, props.icon.class)
      .replace(" svg-string", " svg-node")
      .replace(' xmlns="http://www.w3.org/2000/svg"', "");
    button.insertAdjacentHTML("afterbegin", svg);
    return button;
  }

  function isOverflown(el) {
    return el.scrollWidth > el.clientWidth;
  }

  // FullscreenTableModal: the table itself moves into the modal, a comment
  // keeping its place, and goes back when the modal closes.
  function showFullscreenTable(table, postId) {
    var placeholder;
    Discourse.modal.show({
      className: "fullscreen-table-modal --max",
      title: Discourse.cooked.t("fullscreen_table.view_table"),
      body: function (body) {
        var cooked = document.createElement("div");
        cooked.className = Discourse.cooked.setting("display_footnotes_inline")
          ? "cooked inline-footnotes"
          : "cooked";
        cooked.dataset.refPostId = postId;
        placeholder = document.createComment("fullscreen table");
        table.replaceWith(placeholder);
        cooked.appendChild(table);
        body.appendChild(cooked);
      },
      onClose: function () {
        placeholder.replaceWith(table);
      },
    });
  }

  function generatePopups(tables, postId) {
    tables.forEach(function (table, index) {
      var wrapper = table.parentNode;
      if (wrapper.querySelector(".fullscreen-table-wrapper__buttons")) {
        return;
      }

      var buttonWrapper = document.createElement("div");
      buttonWrapper.classList.add("fullscreen-table-wrapper__buttons");

      wrapper.setAttribute("data-table-index", index);
      wrapper.classList.add("fullscreen-table-wrapper");
      wrapper.insertBefore(buttonWrapper, table);

      if (!isOverflown(wrapper)) {
        return;
      }

      wrapper.classList.add("--has-overflow");

      var expandTableBtn = createButton({
        classes: ["btn-expand-table"],
        title: "fullscreen_table.expand_btn",
        icon: { name: "discourse-expand", class: "expand-table-icon" },
      });
      buttonWrapper.append(expandTableBtn);
      expandTableBtn.addEventListener(
        "click",
        function (event) {
          var t = event.currentTarget.parentElement.nextElementSibling;
          if (t) {
            showFullscreenTable(t, postId);
          }
        },
        false
      );
    });
  }

  Discourse.decorateCooked("table-wrapper", function (el) {
    // onlyStream: a post of the topic's stream.
    var article = el.closest("article[data-post-id]");
    if (!article) {
      return;
    }
    generatePopups(el.querySelectorAll(".md-table table"), article.dataset.postId);
  });
})();
