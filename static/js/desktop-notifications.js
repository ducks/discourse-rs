// Desktop notifications: lib/desktop-notifications.js (init, the primary
// tab, onNotification, confirmNotification), the desktop-notifications
// service's browser half (isNotSupported, the permission getters,
// isSubscribed, enable, disable), subscribe-user-notifications' onAlert
// and NotificationConsentBanner, for a member.
//
// An alert (`/notification-alert/:id`) reaches the page as the live
// stream's `notification-alert` event, its payload as published; the
// stream sends none while the member is in do not disturb
// (canUserReceiveNotifications' isInDoNotDisturb, src/routes/live.rs).
// It is shown through the Notification API when this is not a mobile
// device, init passed (localStorage, Notification, permission not denied
// as the page loaded), this is the primary tab (shown as the page
// loaded or focused since, and no other tab focused after it) and the
// keyValueStore's notifications-disabled isn't "disabled".
//
// registerDesktopNotificationHandler's handlers are called with each
// alert before any of that is asked, as Ember's are, with (data,
// siteSettings, user): the settings shown here read (title) and a user
// answering isInDoNotDisturb. A plugin with alerts of its own (chat's
// /chat/notification-alert) hands them to onNotification.
//
// Push (isPushNotificationsPreferred, isEnabledPush, the service worker)
// is 186's: no worker controls the page, so isPushNotificationsSupported
// is false and enable takes the browser branch for everyone.
(function () {
  "use strict";

  if (!Discourse.once("desktop-notifications")) {
    return;
  }

  var c = {};
  try {
    c = JSON.parse(document.currentScript.dataset.config || "{}");
  } catch (e) {
    return;
  }

  // lib/desktop-notifications' context and lib/push-notifications'.
  var CONTEXT = "discourse_desktop_notifications_";
  var PUSH_CONTEXT = "discourse_push_notifications_";
  var FOCUS_TRACKER = "focus-tracker";
  var DISABLED_KEY = "notifications-disabled";
  var DISMISSED_PROMPT = "dismissed-prompt";

  var primaryTab = false;
  var liveEnabled = false;
  var havePermission = null;
  // messageBus.clientId's stand-in: what marks this tab in the tracker.
  var clientId = Math.random().toString(16).slice(2) + Date.now().toString(16);

  // KeyValueStore: null without localStorage.
  function getItem(context, key) {
    try {
      var value = localStorage.getItem(context + key);
      return value === null ? undefined : value;
    } catch (e) {
      return null;
    }
  }

  function setItem(context, key, value) {
    try {
      localStorage.setItem(context + key, String(value));
    } catch (e) {
      // local storage disabled
    }
  }

  function storageWorks() {
    try {
      localStorage.getItem(CONTEXT + FOCUS_TRACKER);
      return true;
    } catch (e) {
      return false;
    }
  }

  function isNotSupported() {
    return typeof window.Notification === "undefined";
  }

  function notificationsPermission() {
    return isNotSupported() ? "" : Notification.permission;
  }

  function isDeniedPermission() {
    return notificationsPermission() === "denied";
  }

  function isGrantedPermission() {
    return notificationsPermission() === "granted";
  }

  // isEnabledBrowser, read as it is asked for.
  function isEnabledBrowser() {
    return isGrantedPermission() && getItem(CONTEXT, DISABLED_KEY) === "enabled";
  }

  function setIsEnabledBrowser(value) {
    setItem(CONTEXT, DISABLED_KEY, value ? "enabled" : "disabled");
  }

  function interpolate(text, args) {
    return String(text).replace(/%\{(\w+)\}/g, function (all, name) {
      return args[name] == null ? "[missing %{" + name + "} value]" : String(args[name]);
    });
  }

  // i18n("notifications.popup.<name>")
  function popup(name, args) {
    var text = (c.popup || {})[name];
    if (typeof text !== "string") {
      return "[" + (document.documentElement.lang || "en") + ".notifications.popup." + name + "]";
    }
    return interpolate(text, args);
  }

  function confirmNotification() {
    var notification = new Notification(popup("confirm_title", { site_title: c.title }), {
      body: popup("confirm_body", {}),
      icon: c.icon,
      tag: "confirm-subscription",
    });
    var close = function () {
      notification.close();
    };
    notification.addEventListener("click", close);
    setTimeout(function () {
      notification.close();
      notification.removeEventListener("click", close);
    }, 10 * 1000);
  }

  // DesktopNotifications#enable's browser branch: the confirmation is
  // made whatever the answer (the browser shows none unless granted).
  function enable() {
    return new Promise(function (resolve) {
      var answered = false;
      var answer = function (permission) {
        if (answered) {
          return;
        }
        answered = true;
        try {
          confirmNotification();
        } catch (e) {
          // not granted
        }
        if (permission === "granted") {
          setIsEnabledBrowser(true);
        }
        resolve(permission === "granted");
      };
      var asked = Notification.requestPermission(answer);
      if (asked && typeof asked.then === "function") {
        asked.then(answer, function () {
          answer("default");
        });
      }
    });
  }

  function disable() {
    if (isEnabledBrowser()) {
      setIsEnabledBrowser(false);
    }
    return Promise.resolve(true);
  }

  // setupNotifications: the tab focused last is the primary one.
  function setupNotifications() {
    window.addEventListener("storage", function (e) {
      if (e.key === CONTEXT + FOCUS_TRACKER) {
        primaryTab = false;
      }
    });
    window.addEventListener("focus", function () {
      if (!primaryTab) {
        primaryTab = true;
        setItem(CONTEXT, FOCUS_TRACKER, clientId);
      }
    });
    if (typeof document.hidden !== "undefined" && document.hidden) {
      primaryTab = false;
    } else {
      primaryTab = true;
      setItem(CONTEXT, FOCUS_TRACKER, clientId);
    }
  }

  function init() {
    if (!storageWorks() || isNotSupported()) {
      return;
    }
    try {
      if (Notification.permission === "granted") {
        havePermission = true;
      } else if (Notification.permission === "denied") {
        havePermission = false;
        return;
      }
    } catch (e) {
      // defined but not responding: carry on as Ember does
    }
    liveEnabled = true;
    setupNotifications();
  }

  function requestPermission() {
    if (havePermission === true) {
      return Promise.resolve();
    }
    if (havePermission === false) {
      return Promise.reject();
    }
    return new Promise(function (resolve, reject) {
      var answer = function (status) {
        if (status === "granted") {
          resolve();
        } else {
          reject();
        }
      };
      var asked = Notification.requestPermission(answer);
      if (asked && typeof asked.then === "function") {
        asked.then(answer, reject);
      }
    });
  }

  // capabilities.isMobileDevice
  function isMobileDevice() {
    var ua = navigator.userAgent || "";
    return ua.indexOf("Mobile") !== -1 && ua.indexOf("iPad") === -1;
  }

  // DiscourseURL.routeTo: within the topic on the page its route's jump,
  // else the page swapped in as a link's (hx-boost).
  function routeTo(path) {
    var url = (c.base || "") + path;
    if (Discourse.topicRoute && Discourse.topicRoute.topicId()) {
      Discourse.topicRoute.routeToPost(url, { skipIfOnScreen: false });
      return;
    }
    var outlet = document.getElementById("main-outlet");
    if (!window.htmx || !outlet) {
      window.location.assign(url);
      return;
    }
    var link = document.createElement("a");
    link.href = url;
    link.hidden = true;
    outlet.appendChild(link);
    window.htmx.process(link);
    link.click();
  }

  // User#isInDoNotDisturb, as the header avatar carries it.
  function isInDoNotDisturb() {
    var avatar = document.querySelector("#toggle-current-user > img.avatar");
    return !!avatar && "doNotDisturb" in avatar.dataset;
  }

  function canUserReceiveNotifications() {
    return primaryTab && !isInDoNotDisturb() && getItem(CONTEXT, DISABLED_KEY) !== "disabled";
  }

  var desktopNotificationHandlers = [];

  function registerDesktopNotificationHandler(handler) {
    desktopNotificationHandlers.push(handler);
  }

  function onNotification(data) {
    // handlers run first: the permission request below can reject, or
    // stay pending until a background tab is shown again
    desktopNotificationHandlers.forEach(function (handler) {
      handler(data, { title: c.title }, { isInDoNotDisturb: isInDoNotDisturb });
    });
    if (!canUserReceiveNotifications() || !liveEnabled) {
      return;
    }
    var title =
      data.translated_title ||
      popup((c.lookup || {})[data.notification_type], {
        site_title: c.title,
        topic: data.topic_title,
        username: data.username || "",
        group_name: data.group_name,
      });
    var tag = "discourse-notification-" + c.title + "-" + (data.topic_id || 0);
    requestPermission().then(
      function () {
        var notification = new Notification(title, { body: data.excerpt, icon: c.icon, tag: tag });
        notification.addEventListener(
          "click",
          function () {
            routeTo(data.post_url);
            notification.close();
          },
          { once: true }
        );
      },
      function () {
        // permission refused
      }
    );
  }

  document.addEventListener("htmx:sseBeforeMessage", function (event) {
    var message = event.detail;
    if (!message || message.type !== "notification-alert") {
      return;
    }
    event.preventDefault();
    // onAlert
    if (isMobileDevice()) {
      return;
    }
    var data;
    try {
      data = JSON.parse(message.data);
    } catch (e) {
      return;
    }
    if (data) {
      onNotification(data);
    }
  });

  // NotificationConsentBanner: an installed app's member who has neither
  // answered the browser nor the banner.
  function isPwa() {
    return (
      (window.matchMedia && window.matchMedia("(display-mode: standalone)").matches) ||
      !!window.navigator.standalone ||
      document.referrer.indexOf("android-app://") !== -1
    );
  }

  function showNotificationPromptBanner() {
    return (
      c.prompt &&
      !isNotSupported() &&
      isPwa() &&
      Notification.permission !== "denied" &&
      Notification.permission !== "granted" &&
      !isEnabledBrowser() &&
      !getItem(PUSH_CONTEXT, DISMISSED_PROMPT)
    );
  }

  function escapeHtml(text) {
    var div = document.createElement("div");
    div.textContent = text == null ? "" : String(text);
    return div.innerHTML.replace(/"/g, "&quot;");
  }

  function drawBanner() {
    var container = document.getElementById("main-container");
    var drawn = document.querySelector(".consent_banner");
    if (drawn) {
      drawn.parentNode.remove();
    }
    if (!container || !showNotificationPromptBanner()) {
      return;
    }
    var row = document.createElement("div");
    row.className = "row";
    row.innerHTML =
      '<div class="consent_banner alert alert-info"><span>' +
      escapeHtml(c.consent_prompt) +
      ' <button class="btn btn-text btn-link" type="button"><span class="d-button-label">' +
      escapeHtml(c.enable) +
      '</span></button></span><button class="btn no-text btn-icon btn-transparent close" title="' +
      escapeHtml(c.close) +
      '" type="button">' +
      (c.xmark || "") +
      '<span aria-hidden="true">&#8203;</span></button></div>';
    // turnon and dismiss: either answer is kept (setBannerDismissed).
    row.querySelector(".btn-link").addEventListener("click", function () {
      enable();
      setItem(PUSH_CONTEXT, DISMISSED_PROMPT, true);
      row.remove();
    });
    row.querySelector(".close").addEventListener("click", function () {
      setItem(PUSH_CONTEXT, DISMISSED_PROMPT, false);
      row.remove();
    });
    // Before PwaInstallBanner and GlobalNotice.
    var notice = container.querySelector(".global-notice");
    container.insertBefore(row, notice && notice.parentNode === container ? notice : null);
  }

  init();

  Discourse.desktopNotifications = {
    isNotSupported: isNotSupported,
    isDeniedPermission: isDeniedPermission,
    isGrantedPermission: isGrantedPermission,
    // isPushNotificationsPreferred is false (186): the browser's.
    isEnabled: isEnabledBrowser,
    isSubscribed: isEnabledBrowser,
    enable: enable,
    disable: disable,
    registerDesktopNotificationHandler: registerDesktopNotificationHandler,
    onNotification: onNotification,
    isMobileDevice: isMobileDevice,
  };

  document.addEventListener("DOMContentLoaded", function () {
    Discourse.onPage(drawBanner);
  });
})();
