// The image lightbox, as Ember's lib/lightbox.js gives each post: the
// post's a.lightbox images (outside spoilers) are one PhotoSwipe gallery,
// in grid order, reversed for RTL. A click opens it at that image, with a
// "N / total" counter, download (unless prevent_anons_from_downloading_files
// keeps it from a visitor), original image and image info buttons, and
// the title and "WxH size" as its caption. Quoting an image into the
// composer (lib/lightbox/quote-image.js) is not ported.
//
// PhotoSwipe (static/vendor) is imported only for a post with lightboxes:
// its lightbox module when the post is decorated, its core when a gallery
// opens. The script tag carries their urls, whether the viewer may
// download, and the button titles (data-*).
(function () {
  "use strict";

  var Discourse = window.Discourse;
  if (!Discourse || Discourse.lightbox) {
    return;
  }

  var config = document.currentScript.dataset;
  var lightboxModule = null;

  // SELECTORS.DEFAULT_ITEM_SELECTOR
  var ITEM_SELECTOR = "*:not(.spoiler):not(.spoiled) .lightbox";

  var ICONS = {
    download:
      '<path d="M20.5 14.3 17.1 18V10h-2.2v7.9l-3.4-3.6L10 16l6 6.1 6-6.1ZM23 23H9v2h14Z" id="pswp__icn-download"/>',
    image:
      '<path id="pswp__icn-image" d="M8 4C6.23 4 4.8 5.43 4.8 7.2v17.6c0 1.77 1.43 3.2 3.2 3.2h17.6c1.77 0 3.2-1.43 3.2-3.2V7.2c0-1.77-1.43-3.2-3.2-3.2H8zm2.56 4C11.6 8 12.8 9.2 12.8 10.8s-1.2 2.8-2.8 2.8-2.8-1.2-2.8-2.8S9.04 8 10.56 8zm7.2 4.48c0.42 0 0.84 0.21 1.1 0.56l5.44 8.72c0.28 0.46 0.29 1.04 0.03 1.52s-0.74 0.72-1.28 0.72H9.2a1.36 1.36 0 0 1-1.3-0.88c-0.26-0.52-0.22-1.12 0.12-1.6l3.12-4.64c0.28-0.4 0.74-0.64 1.24-0.64s0.96 0.24 1.24 0.64l1.64 2.32 3.76-6.16a1.28 1.28 0 0 1 1.1-0.56z"/>',
    info:
      '<path id="pswp__icn-info" d="M16 28.8C23.07 28.8 28.8 23.07 28.8 16C28.8 8.93 23.07 3.2 16 3.2C8.93 3.2 3.2 8.93 3.2 16C3.2 23.07 8.93 28.8 16 28.8zM14.4 11.2C14.4 10.31 15.115 9.6 16 9.6C16.88 9.6 17.6 10.315 17.6 11.2C17.6 12.085 16.885 12.8 16 12.8C15.115 12.8 14.4 12.085 14.4 11.2zM14 14.4L16.4 14.4C17.065 14.4 17.6 14.935 17.6 15.6L17.6 20L18 20C18.665 20 19.2 20.535 19.2 21.2C19.2 21.865 18.665 22.4 18 22.4L14 22.4C13.335 22.4 12.8 21.865 12.8 21.2C12.8 20.535 13.335 20 14 20L15.2 20L15.2 16.8L14 16.8C13.335 16.8 12.8 16.265 12.8 15.6C12.8 14.935 13.335 14.4 14 14.4z"/>',
  };

  // escapeExpression
  function escapeExpression(s) {
    return String(s).replace(/[&<>"'`]/g, function (c) {
      return {
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        '"': "&quot;",
        "'": "&#x27;",
        "`": "&#x60;",
      }[c];
    });
  }

  // sortLightboxItems: a .d-image-grid's images by data-lightbox-position
  // when all of them have one.
  function sortLightboxItems(items) {
    var result = [];
    var i = 0;
    while (i < items.length) {
      var grid = items[i].closest(".d-image-grid");
      if (!grid) {
        result.push(items[i++]);
        continue;
      }
      var j = i;
      while (j < items.length && items[j].closest(".d-image-grid") === grid) {
        j++;
      }
      var gridItems = items.slice(i, j);
      var positioned = gridItems.every(function (item) {
        var position = item.dataset.lightboxPosition;
        return position !== undefined && position !== "" && Number.isInteger(Number(position));
      });
      if (positioned) {
        gridItems.sort(function (a, b) {
          return Number(a.dataset.lightboxPosition) - Number(b.dataset.lightboxPosition);
        });
      }
      result.push.apply(result, gridItems);
      i = j;
    }
    return result;
  }

  function dimensions(item) {
    var info = item.querySelector(".informations");
    return info ? info.textContent.trim().split(" ")[0] : undefined;
  }

  // Images without data-target-width/height or a "WxH" are loaded first
  // for their size.
  function preload(items) {
    return Promise.all(
      items
        .filter(function (item) {
          var d = item.dataset;
          var hasImageSrc = d.largeSrc || item.getAttribute("href");
          var missingDimensions = !d.targetWidth || !d.targetHeight;
          var dims = dimensions(item);
          var missingMetaData = !(
            dims &&
            dims.split(/x|×/).every(function (n) {
              return !!n;
            })
          );
          return hasImageSrc && missingDimensions && missingMetaData;
        })
        .map(function (item) {
          return new Promise(function (resolve) {
            var img = new Image();
            img.src = item.getAttribute("data-large-src") || item.getAttribute("href");
            img.onload = function () {
              item.setAttribute("data-target-width", img.naturalWidth);
              item.setAttribute("data-target-height", img.naturalHeight);
              resolve();
            };
            img.onerror = resolve;
          });
        })
    );
  }

  function itemData(data) {
    var el = data.element;
    if (!el) {
      return data;
    }

    var width = el.getAttribute("data-target-width");
    var height = el.getAttribute("data-target-height");

    if (el.querySelector("svg[viewBox]") !== null) {
      data.src = "data:image/svg+xml," + encodeURIComponent(el.innerHTML.trim());
      data.inlineSVG = true;
      width = (el.clientWidth || 400) * 10;
      height = (el.clientHeight || 300) * 10;
      // 1x1 placeholder to prevent background flicker on inline SVGs
      data.msrc =
        "data:image/svg+xml,%3Csvg%20xmlns%3D%27http%3A//www.w3.org/2000/svg%27%20width%3D%271%27%20height%3D%271%27/%3E";
    }

    var info = el.querySelector(".informations");
    var imgInfo = (info && info.textContent) || "";
    var imgEl = el.tagName === "IMG" ? el : el.querySelector("img");

    if (!width || !height) {
      var wh = imgInfo.trim().split(" ")[0].split(/x|×/).map(Number);
      width = wh[0];
      height = wh[1];
    }

    // this ensures that cropped images (eg: grid) do not cause jittering when closing
    data.thumbCropped = true;

    data.src = data.src || el.getAttribute("data-large-src");
    data.msrc = data.msrc || (imgEl && (imgEl.currentSrc || imgEl.src));
    data.origSrc =
      (imgEl && imgEl.getAttribute("data-orig-src")) || el.getAttribute("data-orig-src") || null;
    data.title = el.title || (imgEl && (imgEl.alt || imgEl.title)) || null;
    data.aiDescription = (imgEl && imgEl.getAttribute("aria-description")) || null;
    data.base62SHA1 =
      (imgEl && imgEl.getAttribute("data-base62-sha1")) ||
      el.getAttribute("data-base62-sha1") ||
      null;
    data.details = imgInfo;
    data.w = data.width = width;
    data.h = data.height = height;
    data.targetWidth =
      el.getAttribute("data-target-width") || (imgEl && imgEl.getAttribute("width")) || null;
    data.targetHeight =
      el.getAttribute("data-target-height") || (imgEl && imgEl.getAttribute("height")) || null;

    // If thumbnail has different aspect ratio than full image,
    // use the full image as msrc for smooth zoom animation
    if (data.targetWidth && data.targetHeight && data.w && data.h) {
      var thumbRatio = data.targetWidth / data.targetHeight;
      var fullRatio = data.w / data.h;
      if (Math.abs(thumbRatio - fullRatio) / fullRatio > 0.05) {
        data.msrc = data.src;
      }
    }

    return data;
  }

  function registerUi(lightbox, rtl) {
    var ui = lightbox.pswp.ui;

    ui.registerElement({
      name: "caption",
      order: 11,
      isButton: false,
      appendTo: "root",
      html: "",
      onInit: function (caption, pswp) {
        pswp.on("change", function () {
          var data = pswp.currSlide.data;
          if (!data.element || data.inlineSVG) {
            caption.innerHTML = "";
            return;
          }
          var details = data.element.querySelector(".informations");
          var parts = [];
          if (data.title) {
            parts.push("<div class='pswp__caption-title'>" + escapeExpression(data.title) + "</div>");
          }
          if (data.aiDescription) {
            parts.push(
              "<div class='pswp__caption-ai-description'>" +
                escapeExpression(data.aiDescription) +
                "</div>"
            );
          }
          if (details && details.textContent) {
            parts.push("<div class='pswp__caption-details'>" + details.textContent + "</div>");
          }
          caption.innerHTML = parts.join("");
        });
      },
    });

    if (config.canDownload === "true") {
      ui.registerElement({
        name: "download-image",
        order: 7,
        isButton: true,
        tagName: "a",
        title: config.titleDownload,
        html: { isCustomSVG: true, inner: ICONS.download, outlineID: "pswp__icn-download" },
        onInit: function (el, pswp) {
          el.setAttribute("download", "");
          el.setAttribute("target", "_blank");
          el.setAttribute("rel", "noopener");
          pswp.on("change", function () {
            var href = pswp.currSlide.data.element.dataset.downloadHref;
            if (!href) {
              el.style.display = "none";
              return;
            }
            el.href = href;
          });
        },
      });
    }

    ui.registerElement({
      name: "original-image",
      order: 8,
      isButton: true,
      tagName: "a",
      title: config.titleOpen,
      html: { isCustomSVG: true, inner: ICONS.image, outlineID: "pswp__icn-image" },
      onInit: function (el, pswp) {
        el.setAttribute("target", "_blank");
        el.setAttribute("rel", "noopener");
        pswp.on("change", function () {
          el.href = pswp.currSlide.data.src;
        });
      },
    });

    ui.registerElement({
      name: "image-info",
      order: 9,
      isButton: true,
      tagName: "a",
      title: config.titleImageInfo,
      html: { isCustomSVG: true, inner: ICONS.info, outlineID: "pswp__icn-info" },
      onInit: function (el, pswp) {
        pswp.on("change", function () {
          el.style.display = pswp.currSlide.data.details ? "block" : "none";
        });
      },
      onClick: function () {
        lightbox.pswp.element.classList.toggle("pswp--caption-expanded");
      },
    });

    ui.registerElement({
      name: "custom-counter",
      order: 6,
      isButton: false,
      appendTo: "bar",
      onInit: function (el, pswp) {
        pswp.on("change", function () {
          var total = pswp.getNumItems();
          var index = rtl ? total - pswp.currIndex : pswp.currIndex + 1;
          el.textContent = index + " / " + total;
        });
      },
    });
  }

  function initLightbox(PhotoSwipeLightbox, items, rtl) {
    var lightbox = new PhotoSwipeLightbox({
      dataSource: items,
      arrowPrevTitle: config.titlePrevious,
      arrowNextTitle: config.titleNext,
      closeTitle: config.titleClose,
      zoomTitle: config.titleZoom,
      errorMsg: config.titleError,
      showHideAnimationType: "zoom",
      counter: false,
      escKey: false,
      tapAction: function (pt, event) {
        var pswp = lightbox.pswp;
        if (event.target.classList.contains("pswp__img")) {
          pswp && pswp.element && pswp.element.classList.toggle("pswp--ui-visible");
        } else {
          pswp && pswp.close();
        }
      },
      paddingFn: function (viewportSize, data) {
        var mobile = document.documentElement.classList.contains("mobile-device");
        if (viewportSize.x < 1200 || mobile) {
          return { top: 0, bottom: 0, left: 0, right: 0 };
        }
        return { top: 20, bottom: data.title ? 75 : 20, left: 20, right: 20 };
      },
      pswpModule: function () {
        return import(config.photoswipe);
      },
    });

    function keyDownHandler(event) {
      if (event.key !== "Escape") {
        return;
      }
      event.stopPropagation();
      event.preventDefault();
      lightbox.pswp.close();
    }

    lightbox.on("afterInit", function () {
      var meta = lightbox.pswp.currSlide.data.element.querySelector(".meta");
      meta && meta.classList.add("open");
      lightbox.pswp.element.addEventListener("keydown", keyDownHandler);
    });

    lightbox.on("close", function () {
      lightbox.pswp.element.classList.add("pswp--behind-header");
      lightbox.pswp.element.removeEventListener("keydown", keyDownHandler);
    });

    lightbox.on("destroy", function () {
      var meta = lightbox.pswp.currSlide.data.element.querySelector(".meta");
      meta && meta.classList.remove("open");
    });

    lightbox.on("uiRegister", function () {
      registerUi(lightbox, rtl);
    });

    lightbox.addFilter("itemData", itemData);
    lightbox.init();
    return lightbox;
  }

  Discourse.lightbox = function (elem) {
    var rtl = document.documentElement.classList.contains("rtl");
    var items = sortLightboxItems(Array.prototype.slice.call(elem.querySelectorAll(ITEM_SELECTOR)));
    if (!items.length) {
      return;
    }
    if (rtl) {
      items.reverse();
    }

    var ready = null;
    var pending = null;
    // In the capture phase, so the click is the gallery's before htmx
    // boosts the link (its listener is on the link too).
    items.forEach(function (el, index) {
      el.addEventListener(
        "click",
        function (e) {
          e.preventDefault();
          e.stopImmediatePropagation();
          if (ready) {
            ready.loadAndOpen(index);
          } else {
            pending = index;
          }
        },
        true
      );
    });

    lightboxModule = lightboxModule || import(config.lightbox);
    lightboxModule
      .then(function (module) {
        return preload(items).then(function () {
          return module.default;
        });
      })
      .then(function (PhotoSwipeLightbox) {
        ready = initLightbox(PhotoSwipeLightbox, items, rtl);
        if (pending !== null) {
          ready.loadAndOpen(pending);
        }
      });
  };

  Discourse.decorateCooked("lightbox", Discourse.lightbox);
})();
