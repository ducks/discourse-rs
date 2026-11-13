// discourse-lazy-videos' client (initializers/lazy-videos.gjs, onlyStream),
// which the layout loads while lazy_videos_enabled: each
// .lazy-video-container the onebox cooked for a provider whose
// lazy_<provider>_enabled is on becomes p.lazy-video-wrapper holding
// LazyVideo: the thumbnail button, the provider's icon and the title bar,
// swapped for LazyIframe's player by a click, Enter or Space.
(function () {
  "use strict";

  if (!Discourse.once("lazy-videos")) {
    return;
  }

  var PROVIDERS = ["youtube", "vimeo", "tiktok"];

  // lib/lazy-video-attributes.js
  function getVideoAttributes(cooked) {
    var link = cooked.querySelector("a");
    var img = cooked.querySelector("img");
    return {
      url: link ? link.getAttribute("href") : null,
      thumbnail: img ? img.getAttribute("src") : null,
      dominantColor: img ? img.dataset.dominantColor : undefined,
      title: cooked.dataset.videoTitle,
      startTime: cooked.dataset.videoStartTime,
      listId: cooked.dataset.videoListId,
      providerName: cooked.dataset.providerName,
      id: cooked.dataset.videoId,
    };
  }

  // lazy-iframe.gjs' convertToSeconds
  function convertToSeconds(time) {
    var match = time.toString().match(/(?:(\d+)h)?(?:(\d+)m)?(?:(\d+)s)?/);
    var hours = match[1];
    var minutes = match[2];
    var seconds = match[3];
    if (hours || minutes || seconds) {
      var h = parseInt(hours, 10) || 0;
      var m = parseInt(minutes, 10) || 0;
      var s = parseInt(seconds, 10) || 0;
      return h * 3600 + m * 60 + s;
    }
    return time;
  }

  // LazyIframe#iframeSrc
  function iframeSrc(attrs) {
    var id = attrs.id;
    switch (attrs.providerName) {
      case "youtube":
        var url = "https://www.youtube.com/embed/" + id + "?autoplay=1&rel=0";
        if (attrs.listId) {
          url += "&list=" + encodeURIComponent(attrs.listId);
        }
        if (attrs.startTime) {
          url += "&start=" + convertToSeconds(attrs.startTime);
        }
        return url;
      case "vimeo":
        return (
          "https://player.vimeo.com/video/" + id + (id.indexOf("?") !== -1 ? "&" : "?") + "autoplay=1"
        );
      case "tiktok":
        return "https://www.tiktok.com/embed/v2/" + id;
    }
  }

  // Glimmer leaves out an attribute whose value is undefined or null.
  function setAttributes(element, attributes) {
    Object.keys(attributes).forEach(function (key) {
      var value = attributes[key];
      if (value !== undefined && value !== null) {
        element.setAttribute(key, value);
      }
    });
  }

  function element(tag, className, attributes) {
    var el = document.createElement(tag);
    el.className = className;
    setAttributes(el, attributes || {});
    return el;
  }

  // LazyIframe
  function lazyIframe(attrs) {
    var iframe = document.createElement("iframe");
    setAttributes(iframe, {
      allow: "accelerometer; autoplay; encrypted-media; gyroscope; picture-in-picture",
      allowfullscreen: "",
      frameborder: "0",
      scrolling: "no",
      seamless: "seamless",
      src: iframeSrc(attrs),
      title: attrs.title,
    });
    return iframe;
  }

  // LazyVideo
  function lazyVideo(attrs) {
    var provider = attrs.providerName;
    var container = element("div", "lazy-video-container " + provider + "-onebox", {
      "data-provider-name": provider,
      "data-video-id": attrs.id,
      "data-video-list-id": attrs.listId,
      "data-video-start-time": attrs.startTime,
      "data-video-title": attrs.title,
    });

    var thumbnail = element("div", "video-thumbnail " + provider, {
      "aria-label": Discourse.cooked.t("lazy_videos.play_video").replace("%{title}", function () {
        return attrs.title;
      }),
      role: "button",
      // thumbnailStyle
      style:
        attrs.dominantColor && /^[0-9A-Fa-f]+$/.test(attrs.dominantColor)
          ? "background-color: #" + attrs.dominantColor + ";"
          : null,
      tabindex: "0",
    });
    thumbnail.appendChild(
      element("img", provider + "-thumbnail", {
        loading: "lazy",
        src: attrs.thumbnail,
        title: attrs.title,
      })
    );
    thumbnail.appendChild(element("div", "icon " + provider + "-icon"));

    var titleContainer = element("div", "title-container");
    var titleWrapper = element("div", "title-wrapper");
    var titleLink = element("a", "title-link", {
      href: attrs.url,
      rel: "noopener noreferrer",
      target: "_blank",
      title: attrs.title,
    });
    titleLink.textContent = attrs.title === undefined ? "" : attrs.title;
    titleWrapper.appendChild(titleLink);
    titleContainer.appendChild(titleWrapper);

    // loadEmbed; the thumbnail and its listeners go with the swap.
    function loadEmbed() {
      container.classList.add("video-loaded");
      container.replaceChildren(lazyIframe(attrs));
    }
    thumbnail.addEventListener("click", loadEmbed);
    // onKeyPress
    thumbnail.addEventListener("keypress", function (event) {
      if (event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        loadEmbed();
      }
    });

    container.appendChild(thumbnail);
    container.appendChild(titleContainer);
    return container;
  }

  Discourse.decorateCooked("lazy-videos", function (cooked) {
    // onlyStream: a post of the topic's stream.
    if (!cooked.closest("article[data-post-id]")) {
      return;
    }
    cooked.querySelectorAll(".lazy-video-container").forEach(function (container) {
      // One LazyVideo drew already, in content decorated again.
      if (container.parentNode.classList.contains("lazy-video-wrapper")) {
        return;
      }
      var attrs = getVideoAttributes(container);
      if (
        PROVIDERS.indexOf(attrs.providerName) === -1 ||
        !Discourse.cooked.setting("lazy_" + attrs.providerName + "_enabled")
      ) {
        return;
      }
      var wrapper = element("p", "lazy-video-wrapper");
      wrapper.appendChild(lazyVideo(attrs));
      container.replaceWith(wrapper);
    });
  });
})();
