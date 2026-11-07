// extensions/image.js: images (and the video and audio an image's
// `|video` / `|audio` makes), their parse spec over the node tree, the
// serializer, the input rule, and the plugins resolving upload:// sources,
// keeping typed text from replacing a selected image and uploading pasted
// data: images. The node view (media-node-view.gjs) is in
// node-views/image.js. pretty-text/upload-short-url's cache and
// lib/uploads' image extension check are here too, for the node views
// and the composer.
import { getChangedRanges } from "discourse/rich-editor/plugin-utils";
import { MediaNodeView } from "discourse/rich-editor/node-views/image";

var PLACEHOLDER_IMG = "/images/transparent.png";

var ALT_TEXT_REGEX = /^(.*?)(?:\|(\d{1,4}x\d{1,4}))?(?:,\s*(\d{1,3})%)?(?:\|(.*))?$/;

var UPLOAD_TIMEOUT = 30000;

// pretty-text/upload-short-url
export var MISSING = "missing";
var shortUrlCache = {};

export function lookupCachedUploadUrl(shortUrl) {
  return shortUrlCache[shortUrl] || {};
}

export function cacheShortUploadUrl(shortUrl, value) {
  shortUrlCache[shortUrl] = value;
}

export function resetUploadUrlCache() {
  shortUrlCache = {};
}

function csrfHeaders() {
  var headers = {};
  try {
    headers = JSON.parse(document.body.getAttribute("hx-headers") || "{}");
  } catch (e) {
    // No token: the server refuses the request.
  }
  headers["Content-Type"] = "application/x-www-form-urlencoded";
  headers["X-Requested-With"] = "XMLHttpRequest";
  return headers;
}

function basePath() {
  var meta = document.querySelector('meta[name="discourse-base-uri"]');
  return meta ? meta.content : "";
}

// lookupUncachedUploadUrls: POST /uploads/lookup-urls, every url cached
// afterwards (MISSING for those the server does not know). Rejects on a
// failed request, as ajax does.
export function lookupUncachedUploadUrls(urls) {
  urls = urls.filter(Boolean);
  if (urls.length === 0) {
    return Promise.resolve([]);
  }
  var body = urls
    .map(function (url) {
      return "short_urls%5B%5D=" + encodeURIComponent(url);
    })
    .join("&");
  return fetch(basePath() + "/uploads/lookup-urls", {
    method: "POST",
    headers: csrfHeaders(),
    credentials: "same-origin",
    body: body,
  })
    .then(function (r) {
      if (!r.ok) {
        throw new Error("lookup-urls " + r.status);
      }
      return r.json();
    })
    .then(function (uploads) {
      uploads.forEach(function (upload) {
        cacheShortUploadUrl(upload.short_url, { url: upload.url, short_path: upload.short_path });
      });
      urls.forEach(function (url) {
        cacheShortUploadUrl(url, {
          url: lookupCachedUploadUrl(url).url || MISSING,
          short_path: lookupCachedUploadUrl(url).short_path || MISSING,
        });
      });
      return uploads;
    });
}

// lib/uploads.js
function extensionsToArray(exts) {
  return (exts || "")
    .toLowerCase()
    .replace(/[\s.]+/g, "")
    .split("|")
    .filter(function (ext) {
      return !ext.includes("*");
    });
}

function isImagePath(path) {
  return /\.(png|webp|jpe?g|gif|svg|ico|heic|heif|avif|jxl)$/i.test(path);
}

function authorizesAllExtensions(staff, siteSettings) {
  return (
    (siteSettings.authorized_extensions || "").includes("*") ||
    ((siteSettings.authorized_extensions_for_staff || "").includes("*") && staff)
  );
}

export function authorizesOneOrMoreExtensions(staff, siteSettings) {
  if (authorizesAllExtensions(staff, siteSettings)) {
    return true;
  }
  var count = function (exts) {
    return (exts || "").split("|").filter(function (ext) {
      return ext;
    }).length;
  };
  return count(siteSettings.authorized_extensions) > 0 || (count(siteSettings.authorized_extensions_for_staff) > 0 && staff);
}

export function authorizesOneOrMoreImageExtensions(staff, siteSettings) {
  if (authorizesAllExtensions(staff, siteSettings)) {
    return true;
  }
  var exts = extensionsToArray(siteSettings.authorized_extensions).filter(function (ext) {
    return isImagePath("." + ext);
  });
  if (staff) {
    exts = exts.concat(
      extensionsToArray(siteSettings.authorized_extensions_for_staff).filter(function (ext) {
        return isImagePath("." + ext);
      })
    );
  }
  return exts.length > 0;
}

function extractFileExtension(url) {
  var match = url && url.match(/\.([a-z0-9]+)(?:\?|$)/i);
  return (match && match[1]) || "";
}

function buildUploadShortUrl(base62Sha1, url) {
  var ext = extractFileExtension(url);
  return "upload://" + base62Sha1 + (ext ? "." + ext : "");
}

// lib/utilities isNumeric
function isNumeric(input) {
  return /^\d+$/.test(input);
}

function debounce(fn, wait) {
  var timer = null;
  return function () {
    var args = arguments;
    clearTimeout(timer);
    timer = setTimeout(function () {
      fn.apply(null, args);
    }, wait);
  };
}

export default {
  nodeViews: {
    // media-node-view.gjs, not for audio (shouldRender): its toDOM.
    image: function (params) {
      return function (node, view, getPos) {
        if (node.attrs.extras === "audio") {
          return null;
        }
        return new MediaNodeView(node, view, getPos, params);
      };
    },
  },

  nodeSpec: {
    image: {
      inline: true,
      attrs: {
        src: { default: "" },
        alt: { default: null },
        title: { default: null },
        width: { default: null },
        height: { default: null },
        originalSrc: { default: null },
        extras: { default: null },
        scale: { default: null },
        placeholder: { default: null },
      },
      group: "inline",
      draggable: true,
      parseDOM: [
        {
          priority: 70,
          tag: "a.lightbox",
          getAttrs: function (dom) {
            var img = dom.querySelector("img");
            if (!img) {
              return false;
            }

            var href = dom.getAttribute("href");
            var title = dom.getAttribute("title") || img.getAttribute("alt");
            var base62Sha1 = img.dataset.base62Sha1;

            var originalSrc = base62Sha1 ? buildUploadShortUrl(base62Sha1, href) : href;

            return {
              src: img.getAttribute("src"),
              originalSrc: originalSrc,
              alt: title,
              width: img.getAttribute("width"),
              height: img.getAttribute("height"),
            };
          },
        },
        {
          tag: "img[src]",
          getAttrs: function (dom) {
            var src = dom.getAttribute("src");
            if (!src || dom.classList.contains("avatar")) {
              return false;
            }

            var originalSrc =
              dom.dataset.origSrc ||
              (dom.dataset.base62Sha1 && buildUploadShortUrl(dom.dataset.base62Sha1, src));

            var extras = dom.hasAttribute("data-thumbnail") ? "thumbnail" : undefined;
            var title = dom.getAttribute("title");
            var alt = dom.getAttribute("alt");

            return {
              src: src,
              title: title === null ? undefined : title.replace(/\n/g, " "),
              alt: alt === null ? undefined : alt.replace(/\n/g, " "),
              width: dom.getAttribute("width"),
              height: dom.getAttribute("height"),
              originalSrc: originalSrc,
              extras: extras,
              scale: dom.getAttribute("data-scale"),
              placeholder: dom.dataset.placeholder || null,
            };
          },
        },
        {
          tag: "audio source[data-orig-src]",
          getAttrs: function (dom) {
            return {
              originalSrc: dom.getAttribute("data-orig-src"),
              extras: "audio",
            };
          },
        },
        {
          tag: "div[data-video-src]",
          getAttrs: function (dom) {
            var data = dom.dataset;
            return {
              src: data.videoSrc,
              originalSrc: data.origSrc || (data.videoBase62Sha1 && buildUploadShortUrl(data.videoBase62Sha1, data.videoSrc)),
              extras: "video",
            };
          },
        },
        {
          tag: "div.onebox-placeholder-container[data-orig-src]",
          getAttrs: function (dom) {
            return {
              src: "/404",
              alt: dom.dataset.alt || null,
              originalSrc: dom.dataset.origSrc,
              title: dom.dataset.title || null,
              extras: "video",
            };
          },
        },
      ],
      toDOM: function (node) {
        if (node.attrs.extras === "audio") {
          return [
            "audio",
            { preload: "metadata", controls: false, tabindex: -1 },
            ["source", { "data-orig-src": node.attrs.originalSrc }],
          ];
        }

        if (node.attrs.extras === "video") {
          return [
            "div",
            {
              class: "onebox-placeholder-container",
              "data-alt": node.attrs.alt,
              "data-orig-src": node.attrs.originalSrc,
              "data-title": node.attrs.title,
            },
            ["span", { class: "placeholder-icon video" }],
          ];
        }

        var attrs = {
          src: node.attrs.src,
          alt: node.attrs.alt,
          title: node.attrs.title,
          width: node.attrs.width,
          height: node.attrs.height,
          "data-orig-src": node.attrs.originalSrc,
        };

        if (node.attrs.extras === "thumbnail") {
          attrs["data-thumbnail"] = true;
        }

        if (node.attrs.scale !== null) {
          attrs["data-scale"] = node.attrs.scale;
        }

        if (node.attrs.placeholder !== null) {
          attrs["data-placeholder"] = node.attrs.placeholder;
        }

        return ["img", attrs];
      },
    },
  },

  parse: {
    image: {
      node: "image",
      getAttrs: function (token) {
        var match = token.content.match(ALT_TEXT_REGEX);
        var dimensions = match[2];
        var percent = match[3];
        var size = dimensions ? dimensions.split("x") : [];

        return {
          src: token.attrGet("src"),
          title: token.attrGet("title"),
          alt: match[1],
          originalSrc: token.attrGet("data-orig-src"),
          width: size[0],
          height: size[1],
          scale: percent && isNumeric(percent) ? parseInt(percent, 10) : undefined,
          extras: match[4],
        };
      },
    },
  },

  serializeNode: {
    image: function (state, node) {
      if (node.attrs.placeholder) {
        return;
      }

      var src = [node.attrs.originalSrc, node.attrs.src, ""].find(function (value) {
        return value !== null && value !== undefined;
      });

      if (src.startsWith("data:")) {
        state.write("[image]");
        return;
      }

      var alt = (node.attrs.alt || "").replace(/([\\[\]`])/g, "\\$1");
      var scale = node.attrs.scale ? ", " + node.attrs.scale + "%" : "";
      var dimensions =
        node.attrs.width && node.attrs.height ? "|" + node.attrs.width + "x" + node.attrs.height + scale : "";
      var extras = node.attrs.extras ? "|" + node.attrs.extras : "";
      var escapedSrc = src.replace(/[()]/g, "\\$&");
      var title = node.attrs.title ? ' "' + node.attrs.title.replace(/"/g, '\\"') + '"' : "";

      state.write("![" + alt + dimensions + extras + "](" + escapedSrc + title + ")");
    },
  },

  inputRules: function (params) {
    var convertFromMarkdown = params.utils.convertFromMarkdown;
    var NodeSelection = params.pmState.NodeSelection;
    return {
      match: /!\[[^\]]*\]\([^\s]+\)$/,
      handler: function (state, match, start, end) {
        var tr = state.tr;

        return tr
          .replaceWith(start, end, convertFromMarkdown(match[0]))
          .setSelection(NodeSelection.create(tr.doc, start + 2))
          .scrollIntoView();
      },
    };
  },

  plugins: function (params) {
    var Plugin = params.pmState.Plugin;
    var NodeSelection = params.pmState.NodeSelection;
    var TextSelection = params.pmState.TextSelection;
    var getContext = params.getContext;

    var shortUrlResolver = new Plugin({
      state: {
        init: function () {
          return [];
        },
        apply: function (tr, value) {
          var updated = value
            .map(function (entry) {
              return { pos: tr.mapping.map(entry.pos), src: entry.src };
            })
            .filter(function (entry) {
              return entry.pos !== null;
            });

          getChangedRanges(tr).forEach(function (range) {
            tr.doc.nodesBetween(range.new.from, range.new.to, function (node, pos) {
              if (node.type.name === "image" && node.attrs.originalSrc) {
                if (node.attrs.src.endsWith(PLACEHOLDER_IMG)) {
                  updated.push({ pos: pos, src: node.attrs.originalSrc });
                } else {
                  updated = updated.filter(function (u) {
                    return u.src !== node.attrs.originalSrc;
                  });
                }
              }
            });
          });

          return updated;
        },
      },

      view: function () {
        var processUrls = function (view) {
          var unresolvedUrls = shortUrlResolver.getState(view.state);
          if (!unresolvedUrls.length) {
            return;
          }

          var resolvedUrls = {};
          var uncachedSrcs = [];

          unresolvedUrls.forEach(function (entry) {
            var cachedUrl = lookupCachedUploadUrl(entry.src).url;
            if (cachedUrl === MISSING) {
              return;
            } else if (cachedUrl) {
              resolvedUrls[entry.src] = cachedUrl;
            } else if (uncachedSrcs.indexOf(entry.src) < 0) {
              uncachedSrcs.push(entry.src);
            }
          });

          var lookup = uncachedSrcs.length > 0 ? lookupUncachedUploadUrls(uncachedSrcs) : Promise.resolve([]);

          lookup
            .then(function (results) {
              results.forEach(function (result) {
                if (result && result.url && result.short_url) {
                  resolvedUrls[result.short_url] = result.url;
                }
              });

              if (view.isDestroyed) {
                return;
              }

              var tr = view.state.tr.setMeta("addToHistory", false);

              unresolvedUrls.forEach(function (entry) {
                var url = resolvedUrls[entry.src];
                if (!url) {
                  return;
                }

                var node = view.state.doc.nodeAt(entry.pos);
                if (!node || node.type.name !== "image") {
                  return;
                }

                tr.setNodeMarkup(entry.pos, null, Object.assign({}, node.attrs, { src: url }));
              });

              view.dispatch(tr);
            })
            .catch(function () {
              // Unresolved: the placeholders stay.
            });
        };
        var debounced = debounce(processUrls, 100);

        return {
          update: function (view, prevState) {
            if (prevState.doc.eq(view.state.doc)) {
              return;
            }
            debounced(view);
          },
        };
      },
    });

    var avoidTextInputRemoval = new Plugin({
      props: {
        handleTextInput: function (view, from, to, text) {
          var state = view.state;
          var selection = state.selection;

          if (selection instanceof NodeSelection && selection.node.type.name === "image") {
            view.dispatch(state.tr.setSelection(TextSelection.create(state.doc, selection.to - 1)).insertText(text));
            return true;
          }

          return false;
        },
      },
    });

    var dataImageUploader = new Plugin({
      state: {
        init: function () {
          return new Map();
        },
        apply: function (tr, dataURIMap) {
          if (!tr.docChanged) {
            return dataURIMap;
          }

          var updated = new Map();
          tr.doc.descendants(function (node, pos) {
            if (node.type.name === "image" && node.attrs.src && node.attrs.src.startsWith("data:")) {
              var positions = updated.get(node.attrs.src) || new Set();
              positions.add(pos);
              updated.set(node.attrs.src, positions);
            }
          });

          return updated;
        },
      },

      view: function () {
        var context = getContext();
        var appEvents = context.appEvents;
        var siteSettings = context.siteSettings || {};
        var i18n = context.i18n || function (key) {
          return key;
        };
        var pendingUploads = new Set();
        var processingDataURIs = new Set();

        function replaceAll(view, dataURI, text) {
          var tr = view.state.tr;
          var dataURIMap = dataImageUploader.getState(view.state);
          var positions = Array.from(dataURIMap.get(dataURI) || []).sort(function (a, b) {
            return b - a;
          });
          positions.forEach(function (pos) {
            var node = view.state.doc.nodeAt(pos);
            tr.replaceWith(pos, pos + node.nodeSize, view.state.schema.text(text));
          });
          if (tr.docChanged) {
            view.dispatch(tr);
          }
        }

        function uploadDataImage(dataURI, view) {
          if (processingDataURIs.has(dataURI) || pendingUploads.size >= siteSettings.simultaneous_uploads) {
            return;
          }

          var staff = !!(getContext().currentUser && getContext().currentUser.staff);
          if (!authorizesOneOrMoreImageExtensions(staff, siteSettings)) {
            replaceAll(view, dataURI, i18n("composer.image"));
            return;
          }

          if (!appEvents) {
            return;
          }

          processingDataURIs.add(dataURI);

          var mimeMatch = dataURI.match(/data:image\/([^;]+)/);
          var fileExtension = "png";
          if (mimeMatch) {
            var remap = { jpeg: "jpg" };
            var ext = mimeMatch[1].toLowerCase();
            fileExtension = remap[ext] || ext;
          }

          var fileName =
            "image-" + Date.now() + "-" + Math.random().toString(36).substr(2, 9) + "." + fileExtension;

          pendingUploads.add(fileName);

          var handleSuccess;
          var handleError;
          var cleanup = function () {
            pendingUploads.delete(fileName);
            processingDataURIs.delete(dataURI);
            appEvents.off("composer:upload-success", handleSuccess);
            appEvents.off("composer:upload-error", handleError);
            processQueuedUploads(view);
          };

          handleError = function (uploadedFileName) {
            if ((uploadedFileName === fileName || !uploadedFileName) && pendingUploads.has(fileName)) {
              replaceAll(view, dataURI, "[Upload failed]");
              cleanup();
            }
          };

          handleSuccess = function (uploadedFileName, upload) {
            if (uploadedFileName === fileName && pendingUploads.has(fileName)) {
              if (!(upload && (upload.short_url || upload.url))) {
                return handleError(fileName);
              }

              var tr = view.state.tr;
              var dataURIMap = dataImageUploader.getState(view.state);
              (dataURIMap.get(dataURI) || []).forEach(function (pos) {
                var node = view.state.doc.nodeAt(pos);
                tr.setNodeMarkup(
                  pos,
                  null,
                  Object.assign({}, node.attrs, {
                    alt: node.attrs.alt || i18n("upload_selector.default_image_alt_text"),
                    src: upload.url || upload.short_url,
                    originalSrc: upload.short_url || upload.url,
                  })
                );
              });

              if (tr.docChanged) {
                view.dispatch(tr);
              }
              cleanup();
            }
          };

          fetch(dataURI)
            .then(function (res) {
              return res.blob();
            })
            .then(function (blob) {
              var file = new File([blob], fileName, { type: blob.type });
              file.id = fileName;

              appEvents.on("composer:upload-success", handleSuccess);
              appEvents.on("composer:upload-error", handleError);

              appEvents.trigger("composer:add-files", [file], { skipPlaceholder: true });

              setTimeout(function () {
                if (pendingUploads.has(fileName)) {
                  handleError(fileName);
                }
              }, UPLOAD_TIMEOUT);
            });
        }

        function processQueuedUploads(view) {
          var dataURIMap = dataImageUploader.getState(view.state);
          var keys = Array.from(dataURIMap.keys());
          for (var i = 0; i < keys.length; i++) {
            if (!processingDataURIs.has(keys[i])) {
              if (pendingUploads.size >= siteSettings.simultaneous_uploads) {
                break;
              }
              uploadDataImage(keys[i], view);
            }
          }
        }

        return {
          update: function (view) {
            processQueuedUploads(view);
          },
        };
      },
    });

    return [shortUrlResolver, avoidTextInputRemoval, dataImageUploader];
  },
};
