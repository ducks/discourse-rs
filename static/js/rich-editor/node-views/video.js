// components/video-node-view.gjs: a `|video` image as a player. The video
// and its generated thumbnail (the extensionless short url) are looked up
// first; the source is set only once playback is requested, and a paused
// video leaves pointer handling to the editor until it plays.
import { NodeSelection } from "prosemirror-state";
import { iconHtml } from "discourse/rich-editor/toolbar";
import {
  lookupCachedUploadUrl,
  lookupUncachedUploadUrls,
  MISSING,
} from "discourse/rich-editor/extensions/image";

var UNRESOLVED_MEDIA_PATHS = ["/404", "/images/transparent.png"];

function resolvedUrl(url) {
  return url &&
    url !== MISSING &&
    !UNRESOLVED_MEDIA_PATHS.some(function (path) {
      return url.endsWith(path);
    })
    ? url
    : null;
}

function videoThumbnailShortUrl(url) {
  var match = url && url.match(/^(upload:\/\/[a-zA-Z0-9]+)(?:\.[a-z0-9]+)?$/i);
  return match ? match[1] : undefined;
}

export function VideoNodeView(owner) {
  var self = this;
  this.owner = owner;
  this.dom = owner.dom;
  var context = owner.params.getContext();
  this.i18n =
    context.i18n ||
    function (key) {
      return key;
    };
  this.a11y = context.a11y;
  this.error = null;
  this.isActivated = false;
  this.isPlaybackRequested = false;
  this.poster = null;
  this.source = null;
  this.loadId = 0;
  this.playId = 0;
  this.playPending = false;
  this.originalSrc = owner.node.attrs.originalSrc;

  this.dom.classList.remove("composer-image-node");
  this.dom.classList.add("composer-video-node");

  var video = (this.video = document.createElement("video"));
  video.contentEditable = "false";
  video.controls = true;
  video.setAttribute("playsinline", "");
  video.setAttribute("preload", "metadata");
  video.addEventListener("click", function () {
    self.selectVideo();
  });
  video.addEventListener("error", function () {
    self.handlePlaybackError();
  });
  video.addEventListener("playing", function () {
    self.activateVideo();
  });
  this.dom.appendChild(video);

  var play = (this.playButton = document.createElement("button"));
  play.type = "button";
  play.className = "btn no-text btn-icon btn-flat composer-video-node__play";
  play.title = this.i18n("play_video");
  play.innerHTML = iconHtml("play") + '<span aria-hidden="true">&#8203;</span>';
  play.addEventListener("click", function (event) {
    event.preventDefault();
    self.playVideo();
  });

  this.render();
  this.load(owner.node);
}

VideoNodeView.prototype.render = function () {
  var node = this.owner.node;
  var video = this.video;
  if (node.attrs.alt === null || node.attrs.alt === undefined) {
    video.removeAttribute("aria-label");
  } else {
    video.setAttribute("aria-label", node.attrs.alt);
  }
  video.className = "composer-video-node__video" + (this.isActivated ? "" : " is-inactive");
  if (this.poster) {
    video.setAttribute("poster", this.poster);
  } else {
    video.removeAttribute("poster");
  }
  var src = this.isPlaybackRequested ? this.source : null;
  if (src) {
    if (video.getAttribute("src") !== src) {
      video.setAttribute("src", src);
    }
  } else {
    video.removeAttribute("src");
  }

  if (this.error) {
    if (!this.errorEl) {
      this.errorEl = document.createElement("div");
      this.errorEl.className = "notice error composer-video-node__error";
      this.dom.insertBefore(this.errorEl, this.video.nextSibling);
    }
    this.errorEl.innerHTML = iconHtml("triangle-exclamation") + " ";
    this.errorEl.appendChild(document.createTextNode(this.i18n(this.error)));
  } else if (this.errorEl) {
    this.errorEl.remove();
    this.errorEl = null;
  }

  if (this.isActivated) {
    this.playButton.remove();
  } else if (!this.playButton.isConnected) {
    this.dom.appendChild(this.playButton);
  }
};

VideoNodeView.prototype.activateVideo = function () {
  this.error = null;
  this.isActivated = true;
  this.render();
};

VideoNodeView.prototype.handlePlaybackError = function () {
  if (this.source && !this.destroyed) {
    this.setError("cannot_render_video", this.playPending ? "assertive" : "polite");
  }
};

VideoNodeView.prototype.playVideo = function () {
  var self = this;
  var playId = ++this.playId;
  var loaded = this.source ? Promise.resolve(true) : this.load(this.owner.node, { retryMissing: true });

  return loaded.then(function (ok) {
    if (!ok || playId !== self.playId) {
      return;
    }
    self.error = null;
    self.isPlaybackRequested = true;
    self.render();
    self.playPending = true;
    var playing;
    try {
      playing = Promise.resolve(self.video.play());
    } catch (e) {
      playing = Promise.reject(e);
    }
    return playing
      .catch(function () {
        if (playId === self.playId) {
          self.setError("cannot_render_video", "assertive");
        }
      })
      .then(function () {
        if (playId === self.playId) {
          self.playPending = false;
        }
      });
  });
};

VideoNodeView.prototype.selectVideo = function () {
  var view = this.owner.view;
  var pos = this.owner.getPos();
  view.dispatch(view.state.tr.setSelection(NodeSelection.create(view.state.doc, pos)));
  view.focus();
};

VideoNodeView.prototype.selectNode = function () {
  this.dom.classList.add("ProseMirror-selectednode");
};

VideoNodeView.prototype.deselectNode = function () {
  this.dom.classList.remove("ProseMirror-selectednode");
};

VideoNodeView.prototype.stopEvent = function (event) {
  if (["dragstart", "dragover", "dragend", "drop", "dragleave"].indexOf(event.type) >= 0) {
    return false;
  }
  return (
    event.target instanceof HTMLVideoElement ||
    !!(event.target.closest && event.target.closest(".composer-video-node__play"))
  );
};

VideoNodeView.prototype.update = function (node) {
  if (node.attrs.originalSrc !== this.originalSrc) {
    this.isActivated = false;
    this.isPlaybackRequested = false;
    this.playId++;
    this.playPending = false;
    this.originalSrc = node.attrs.originalSrc;
  }
  this.render();
  this.load(node);
};

VideoNodeView.prototype.load = function (node, opts) {
  var self = this;
  var retryMissing = !!(opts && opts.retryMissing);
  var loadId = ++this.loadId;
  this.error = null;
  this.source = resolvedUrl(node.attrs.src);
  this.poster = null;
  this.render();

  var sourceShortUrl = node.attrs.originalSrc;
  var posterShortUrl = videoThumbnailShortUrl(sourceShortUrl);
  var shortUrls = [];
  [sourceShortUrl, posterShortUrl].forEach(function (url) {
    var cachedUrl = lookupCachedUploadUrl(url).url;
    if (url && (!cachedUrl || (retryMissing && cachedUrl === MISSING)) && shortUrls.indexOf(url) < 0) {
      shortUrls.push(url);
    }
  });

  var promise = shortUrls.length > 0 ? lookupUncachedUploadUrls(shortUrls) : Promise.resolve();

  return promise.then(
    function () {
      if (loadId !== self.loadId || self.destroyed) {
        return false;
      }

      self.source = resolvedUrl(lookupCachedUploadUrl(sourceShortUrl).url) || self.source;
      self.poster = resolvedUrl(lookupCachedUploadUrl(posterShortUrl).url) || self.poster;

      // The server resolves the extensionless short URL to the video's own
      // upload when no thumbnail was generated. A video is not a usable
      // poster.
      if (self.poster && self.poster === self.source) {
        self.poster = null;
      }

      if (!self.source) {
        self.setError("invalid_video_url", "polite");
        return false;
      }

      self.render();
      return true;
    },
    function () {
      if (loadId === self.loadId && !self.destroyed) {
        self.setError("invalid_video_url", "polite");
      }
      return false;
    }
  );
};

VideoNodeView.prototype.setError = function (key, announcementLevel) {
  if (this.error === key || this.destroyed) {
    return;
  }
  this.error = key;
  this.render();
  if (announcementLevel && this.a11y && this.a11y.announce) {
    this.a11y.announce(this.i18n(key), announcementLevel);
  }
};

VideoNodeView.prototype.destroy = function () {
  this.destroyed = true;
  this.loadId++;
  this.playId++;
  this.playPending = false;
  this.dom.classList.remove("composer-video-node", "ProseMirror-selectednode");
  this.dom.classList.add("composer-image-node");
};
