// Avatars at the screen's pixel density, as Ember asks for them: its
// avatarImg (discourse/lib/avatar-utils) draws width and height at the
// size's pixels (translateSize: tiny 24, medium 48, huge 144...) and the
// src at getRawAvatarSize of that, the size times the device pixel ratio
// rounded up to the next of avatar_sizes (the largest when none is). No
// srcset: a DPR 3 phone's tiny avatar is width="24" with a 72.png src.
// The server can't know the ratio, so it draws the src for a ratio of 1
// and this script redraws it in the browser.
//
// It loads in the head, before the body, and watches the document, so an
// avatar the parser (or an htmx swap, a bus update, a fetched user card or
// menu) inserts is redrawn in the microtask after its insertion.
//
// An avatar is an img with a width whose src is an avatar URL (user_avatar,
// letter_avatar, letter_avatar_proxy) at the size the server draws for that
// width: the width itself or its size at a ratio of 1. Cooked content keeps
// its src (the cooked quote avatars are drawn at 2x when cooked, Ember
// leaves them), as do the user API key pages, whose avatar Ember draws at
// a fixed 24 (user-api-key-new's avatarUrl).
//
// avatar_sizes comes with the script tag (data-sizes).
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.avatarSrc) {
    return;
  }

  var sizes = (document.currentScript.dataset.sizes || "")
    .split("|")
    .map(function (s) {
      return parseInt(s, 10);
    })
    .filter(function (n) {
      return !isNaN(n);
    })
    .sort(function (a, b) {
      return a - b;
    });

  var AVATAR =
    /(\/(?:user_avatar\/[^/]+\/[^/]+|letter_avatar\/[^/]+|letter_avatar_proxy\/v\d+\/letter\/[^/]+\/[^/]+)\/)(\d+)(\/|\.png)/;
  var FIXED = ".cooked, .chat-message-text, .authorize-api-key";

  // getRawSize: 1 up to a ratio of 1.1, 2 below 2.1, else 3.
  function rawSize(size, ratio) {
    var scale = 1;
    if (ratio > 1.1 && ratio < 2.1) {
      scale = 2;
    } else if (ratio >= 2.1) {
      scale = 3;
    }
    return size * scale;
  }

  // getRawAvatarSize
  function rawAvatarSize(size, ratio) {
    var raw = rawSize(size, ratio);
    for (var i = 0; i < sizes.length; i++) {
      if (sizes[i] >= raw) {
        return sizes[i];
      }
    }
    return sizes[sizes.length - 1];
  }

  // The src Ember draws for an avatar the server drew, or null when the
  // img is not one.
  function avatarSrc(img, ratio) {
    if (!sizes.length) {
      return null;
    }
    var width = parseInt(img.getAttribute("width"), 10);
    var src = img.getAttribute("src");
    if (isNaN(width) || !src) {
      return null;
    }
    var m = AVATAR.exec(src);
    if (!m) {
      return null;
    }
    var drawn = parseInt(m[2], 10);
    if (drawn !== width && drawn !== rawAvatarSize(width, 1)) {
      return null;
    }
    if (img.closest(FIXED)) {
      return null;
    }
    var want = rawAvatarSize(width, ratio);
    if (want === drawn) {
      return null;
    }
    return src.slice(0, m.index) + m[1] + want + m[3] + src.slice(m.index + m[0].length);
  }

  function redraw(root) {
    var ratio = window.devicePixelRatio || 1;
    var imgs = root.tagName === "IMG" ? [root] : root.querySelectorAll("img[width][src]");
    Array.prototype.forEach.call(imgs, function (img) {
      var src = avatarSrc(img, ratio);
      if (src) {
        img.setAttribute("src", src);
      }
    });
  }

  new MutationObserver(function (records) {
    records.forEach(function (record) {
      Array.prototype.forEach.call(record.addedNodes, function (node) {
        if (node.nodeType === 1) {
          redraw(node);
        }
      });
    });
  }).observe(document.documentElement, { childList: true, subtree: true });

  Discourse.avatarSrc = avatarSrc;
  redraw(document);
})();
