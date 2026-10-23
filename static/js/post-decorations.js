// Ember's post decorators for image grids and tables
// (instance-initializers/post-decorations.js) and uploaded videos
// (instance-initializers/video-placeholder.js):
//
//   image grid: each div.d-image-grid not in carousel mode is laid out in
//       columns by lib/columns.js. The carousel (lib/image-grid-carousel.js)
//       and the lightbox aren't ported.
//   table-wrapper (onlyStream): each .md-table of a post gets the buttons
//       row, and when it overflows, the button that shows the table in a
//       FullscreenTableModal. The pencil that opens the spreadsheet editor
//       (`--editable`, for a post the viewer can edit) isn't ported.
//   video-placeholder (onlyStream): below.
//
// Its audio decorator only keeps a playing post from being cloaked
// (api.preventCloak); the pages do not cloak posts, so it has nothing to do.
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

  // instance-initializers/video-placeholder.js (onlyStream): each
  // .video-placeholder-container cooking left gets its thumbnail as the
  // background and a play button; a click swaps in the <video>, shown
  // once it can play.
  var ua = navigator.userAgent;
  var isSafari = /^((?!chrome|android).)*safari/i.test(ua);
  var isIOS =
    /iPhone|iPad|iPod/.test(ua) ||
    (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1);

  // sanitizeUrl: an http(s) or root-relative url that sanitizing leaves
  // as it is, else null.
  function sanitizeUrl(url) {
    try {
      var parsed = new URL(url, window.location.origin);
      if (
        ["http:", "https:"].indexOf(parsed.protocol) !== -1 ||
        url.startsWith("/")
      ) {
        if (
          url &&
          url.trim() !== "" &&
          url.indexOf("<") === -1 &&
          url.indexOf(">") === -1 &&
          url.indexOf('"') === -1
        ) {
          return url;
        }
      }
    } catch (e) {
      window.console && console.warn("Invalid URL encountered:", url, e.message);
    }
    return null;
  }

  function handleVideoPlaceholderClick(event) {
    var parentDiv = event.target.closest(".video-placeholder-container");
    var wrapper = parentDiv.querySelector(".video-placeholder-wrapper");
    var overlay = wrapper.querySelector(".video-placeholder-overlay");

    parentDiv.style.cursor = "";
    overlay.innerHTML = "<div class='spinner'></div>";

    var videoSrc = sanitizeUrl(parentDiv.dataset.videoSrc);
    var origSrc = sanitizeUrl(parentDiv.dataset.origSrc);
    var dataOrigSrcAttr = origSrc !== null ? 'data-orig-src="' + origSrc + '"' : "";

    if (videoSrc === null) {
      var existingNotice = wrapper.querySelector(".notice.error");
      if (existingNotice) {
        existingNotice.remove();
      }
      var notice = document.createElement("div");
      notice.className = "notice error";
      notice.innerHTML =
        Discourse.cooked.icon("triangle-exclamation") +
        " " +
        Discourse.cooked.t("invalid_video_url");
      wrapper.appendChild(notice);
      overlay.innerHTML = Discourse.cooked.icon("play");
      parentDiv.style.cursor = "pointer";
      parentDiv.addEventListener("click", handleVideoPlaceholderClick, { once: true });
      return;
    }

    parentDiv.insertAdjacentHTML(
      "beforeend",
      '\n        <video width="100%" height="100%" preload="metadata" controls style="display:none">\n' +
        '          <source src="' + videoSrc + '" ' + dataOrigSrcAttr + ">\n" +
        '          <a href="' + videoSrc + '">' + videoSrc + "</a>\n" +
        "        </video>"
    );
    parentDiv.classList.add("video-container");

    var video = parentDiv.querySelector("video");
    if ((isSafari || isIOS) && video.querySelector("source")) {
      video.load();
    }

    video.addEventListener("loadeddata", function () {
      setTimeout(function () {
        if (video.videoWidth === 0 || video.videoHeight === 0) {
          var codecNotice = document.createElement("div");
          codecNotice.className = "notice";
          codecNotice.innerHTML =
            Discourse.cooked.icon("triangle-exclamation") +
            " " +
            Discourse.cooked.t("cannot_render_video");
          parentDiv.appendChild(codecNotice);
        }
      }, 500);
    });

    video.addEventListener("canplay", function () {
      if (isIOS) {
        setTimeout(function () {
          video.play();
        }, 100);
      } else {
        video.play();
      }
      wrapper.remove();
      video.style.display = "";
      parentDiv.classList.remove("video-placeholder-container");
      parentDiv.style.backgroundImage = "none";
    });
  }

  Discourse.decorateCooked("video-placeholder", function (el) {
    if (!el.closest("article[data-post-id]")) {
      return;
    }
    el.querySelectorAll(".video-placeholder-container").forEach(function (container) {
      if (container.dataset.thumbnailSrc) {
        var thumbnail = new Image();
        thumbnail.onload = function () {
          container.style.backgroundImage = "url('" + thumbnail.src + "')";
        };
        thumbnail.src = container.dataset.thumbnailSrc;
      }
      var wrapper = document.createElement("div");
      var overlay = document.createElement("div");
      wrapper.classList.add("video-placeholder-wrapper");
      container.appendChild(wrapper);
      overlay.classList.add("video-placeholder-overlay");
      container.style.cursor = "pointer";
      container.addEventListener("click", handleVideoPlaceholderClick, { once: true });
      overlay.innerHTML = Discourse.cooked.icon("play");
      wrapper.appendChild(overlay);
    });
  });
})();
