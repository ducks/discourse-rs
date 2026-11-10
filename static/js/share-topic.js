// The share modal (components/modal/share-topic.gjs) and what it shares
// with the quote toolbar: lib/sharing.js over the sharing-sources
// initializer's sources (X, Facebook, email).
//
//   Discourse.sharing.activeSources(linksSetting, privateContext)
//       Sharing.activeSources: the sources a setting lists
//       ("twitter|facebook|email"), in its order; in a private context
//       only those that show there (showInPrivateContext).
//   Discourse.sharing.shareSource(source, { url, title, quote, setting })
//       Sharing.shareSource: the source's URL in a popup, a new tab or,
//       for email, this one. setting(name) reads the site settings the
//       sources look at (facebook_app_id, title).
//   Discourse.shareTopic.show()
//       the modal for the page's topic: its link (topic.shareUrl, the
//       footer share button's data-share-url) in a read-only input,
//       selected on desktop, with a copy button, and the share_links
//       sources. The settings, strings and topic are the page's
//       #quote-button-config.
//
// Not ported yet: the invite, new topic and plugin outlet buttons, the
// restricted category's warning and the modal for a post.
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (!Discourse.once("share-topic")) {
    return;
  }

  // DCopyButton's restore delay.
  var COPIED_MS = 3000;

  var SOURCES = {
    twitter: {
      icon: "fab-x-twitter",
      title: "share.twitter",
      popupHeight: 265,
      popup: true,
      url: function (link, title, quote) {
        var text = quote ? '"' + quote + '" -- ' : title;
        return "http://x.com/intent/tweet?url=" + encodeURIComponent(link) + "&text=" + encodeURIComponent(text);
      },
    },
    facebook: {
      icon: "fab-facebook",
      title: "share.facebook",
      popup: true,
      url: function (link, title, quote, setting) {
        var app = setting("facebook_app_id");
        var base = app
          ? "https://www.facebook.com/dialog/share?app_id=" + app + "&quote=" + encodeURIComponent(quote || "") + "&href="
          : "https://www.facebook.com/sharer.php?u=";
        return base + encodeURIComponent(link);
      },
    },
    email: {
      icon: "envelope",
      title: "share.email",
      privateContext: true,
      url: function (link, title, quote, setting) {
        var body = quote ? quote + " \n\n " + link : link;
        return (
          "mailto:?to=&subject=" +
          encodeURIComponent("[" + setting("title") + "] " + title) +
          "&body=" +
          encodeURIComponent(body)
        );
      },
    },
  };

  function activeSources(linksSetting, privateContext) {
    return String(linksSetting || "")
      .split("|")
      .map(function (id) {
        return SOURCES[id] ? Object.assign({ id: id }, SOURCES[id]) : null;
      })
      .filter(function (s) {
        return s && (!privateContext || s.privateContext);
      });
  }

  function shareSource(source, data) {
    var link = source.url(data.url, data.title, data.quote || "", data.setting);
    if (source.popup) {
      window.open(
        link,
        "",
        "menubar=no,toolbar=no,resizable=yes,scrollbars=yes,width=600,height=" + (source.popupHeight || 315)
      );
    } else if (source.id === "email") {
      window.location.href = link;
    } else {
      window.open(link, "_blank");
    }
  }

  Discourse.sharing = { activeSources: activeSources, shareSource: shareSource };

  function readConfig() {
    var el = document.getElementById("quote-button-config");
    if (!el) {
      return null;
    }
    try {
      return JSON.parse(el.textContent);
    } catch (e) {
      window.console && console.error("quote-button-config", e);
      return null;
    }
  }

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function icon(name) {
    return (
      '<svg class="fa d-icon d-icon-' + name +
      ' svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name + '"></use></svg>'
    );
  }

  // The footer's share button carries topic.shareUrl; the footer waits in
  // a template until the last post is loaded.
  function shareUrl() {
    var selector = "#topic-footer-button-share-and-invite";
    var button = document.querySelector(selector);
    var waiting = document.querySelector("template.loaded-all-posts");
    if (!button && waiting) {
      button = waiting.content.querySelector(selector);
    }
    return button ? new URL(button.dataset.shareUrl, location.href).href : null;
  }

  // DCopyButton: the input's value to the clipboard, the icon a check for
  // a while.
  function copy(button, input) {
    if (!navigator.clipboard) {
      return;
    }
    navigator.clipboard.writeText(input.value).then(
      function () {
        button.classList.add("ok");
        button.querySelector("svg").outerHTML = icon("check");
        clearTimeout(button.restore);
        button.restore = setTimeout(function () {
          button.classList.remove("ok");
          button.querySelector("svg").outerHTML = icon("copy");
        }, COPIED_MS);
      },
      function () {}
    );
  }

  function show() {
    var config = readConfig();
    var url = shareUrl();
    if (!config || !url) {
      return;
    }
    var settings = config.settings || {};
    var strings = config.strings || {};
    var topic = config.topic;
    function setting(name) {
      return settings[name];
    }
    function t(key) {
      return strings[key] === undefined ? key : strings[key];
    }
    var privateContext =
      !!settings.login_required || topic.privateMessage || topic.invisible || topic.readRestricted;
    var sources = activeSources(settings.share_links, privateContext);

    Discourse.modal.show({
      className: "share-topic-modal",
      title: t("topic.share.title"),
      body: function (body) {
        body.innerHTML =
          '<form><div class="input-group invite-link"><label for="invite-link">' +
          escapeHtml(t("topic.share.instructions")) +
          '</label><div class="link-share-container">' +
          '<input class="ember-text-field invite-link" id="invite-link" name="invite-link" readonly size="200" type="text">' +
          '<button class="btn no-text btn-icon copy-button btn-primary" aria-label="' +
          escapeHtml(t("share.url")) +
          '" type="button">' + icon("copy") + '<span aria-hidden="true">&#8203;</span></button>' +
          '<span aria-live="polite" class="sr-only"></span></div></div>' +
          '<div class="link-share-actions"><div class="sources">' +
          sources
            .map(function (source) {
              return (
                '<button class="btn no-text btn-icon btn-default share-' + source.id + '" title="' +
                escapeHtml(t(source.title)) + '" type="button">' + icon(source.icon) +
                '<span aria-hidden="true">&#8203;</span></button>'
              );
            })
            .join("") +
          "</div></div></form>";
        var form = body.querySelector("form");
        var input = form.querySelector("input.invite-link");
        input.value = url;
        // Enter in a form's only field would submit it.
        form.addEventListener("submit", function (event) {
          event.preventDefault();
        });
        form.addEventListener("click", function (event) {
          var button = event.target.closest("button");
          if (!button) {
            return;
          }
          if (button.classList.contains("copy-button")) {
            copy(button, input);
            return;
          }
          var index = Array.prototype.indexOf.call(form.querySelectorAll(".sources > button"), button);
          if (sources[index]) {
            shareSource(sources[index], { url: url, title: topic.title, setting: setting });
          }
        });
      },
    });

    // _selectUrl: if the input is auto-focused on mobile, iOS requires two
    // taps of the copy button.
    var link = document.querySelector(".share-topic-modal input.invite-link");
    if (link && !document.documentElement.classList.contains("mobile-view")) {
      link.setSelectionRange(0, url.length);
      link.focus();
    }
  }

  Discourse.shareTopic = { show: show };
})();
