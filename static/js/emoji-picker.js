// The emoji picker (EmojiPicker's content in a float-kit DMenu): the
// emoji of /emojis.json by section, with the context's frequently used
// first, a nav to each section, the filter (emojiSearch over the names and
// /emojis/search-aliases.json), the skin tone menu (DiversityMenu) and the
// keyboard moves between emoji. What it draws and its strings come from
// the page's template.emoji-picker-template.
//
// The browser's emoji usage and skin tone (EmojiStore) live here too, in
// its localStorage keys:
//
//   Discourse.emojiStore.diversity(), .track(emoji, context),
//     .favorites(context)
//   Discourse.emojiPicker.open(trigger, { context, term, onSelect, onClose,
//     maxWidth })
//   Discourse.emojiPicker.close(), .shortcutReaction(text)
(function () {
  "use strict";

  if (!Discourse.once("emoji-picker")) {
    return;
  }

  var STORE = "discourse_emoji_reaction_";
  var MAX_DISPLAYED = 20;
  var MAX_TRACKED = MAX_DISPLAYED * 2;
  var INPUT_DELAY = 250;
  var MENU_OFFSET = 10;
  var MENU_MAX_WIDTH = 400;
  // FLOAT_UI_PLACEMENTS, the fallbacks of the default bottom-start.
  var PLACEMENTS = [
    "bottom-start",
    "top",
    "top-start",
    "top-end",
    "right",
    "right-start",
    "right-end",
    "bottom",
    "bottom-end",
    "left",
    "left-start",
    "left-end",
  ];

  // KeyValueStore
  function storeGet(key) {
    try {
      var value = window.localStorage.getItem(STORE + key);
      return value === null ? null : JSON.parse(value);
    } catch (e) {
      return null;
    }
  }

  function storeSet(key, value) {
    try {
      window.localStorage.setItem(STORE + key, JSON.stringify(value));
    } catch (e) {
      // No storage: nothing is remembered.
    }
  }

  function diversity() {
    return Number(storeGet("emojiSelectedDiversity") || 1);
  }

  function hasTone(emoji) {
    return /:t[1-6]:?$/.test(emoji);
  }

  // EmojiStore#trackEmojiForContext
  function track(emoji, context) {
    var key = context + "_emojiUsage";
    var recent = storeGet(key) || [];
    recent.unshift(emoji.replace(/(^:)|(:$)/g, ""));
    recent.length = Math.min(recent.length, MAX_TRACKED);
    storeSet(key, recent);
  }

  // EmojiStore#favoritesForContext: by use, the skin tone on tonable ones
  // not toned yet.
  function favorites(context, tonable) {
    var counters = {};
    (storeGet(context + "_emojiUsage") || []).forEach(function (emoji) {
      counters[emoji] = (counters[emoji] || 0) + 1;
    });
    var tone = diversity();
    return Object.keys(counters)
      .sort(function (a, b) {
        return counters[b] - counters[a];
      })
      .slice(0, MAX_DISPLAYED)
      .map(function (emoji) {
        if (tone === 1 || hasTone(emoji) || !tonable(emoji)) {
          return emoji;
        }
        return emoji + ":t" + tone;
      });
  }

  function resetContext(context) {
    storeSet(context + "_emojiUsage", []);
  }

  Discourse.emojiStore = {
    diversity: diversity,
    track: track,
    // `tonable` says whether a name takes a skin tone; by default, what
    // /emojis.json says once loaded.
    favorites: function (context, tonable) {
      return favorites(
        context,
        tonable ||
          function (emoji) {
            return !!tonableNames[emoji.split(":")[0]];
          }
      );
    },
  };

  var list = null;
  var listRequest = null;
  var tonableNames = {};
  var urls = {};
  var searchAliases = null;
  var toSearch = null;

  function config() {
    var tpl = document.querySelector("template.emoji-picker-template");
    if (!tpl) {
      return null;
    }
    return {
      tpl: tpl,
      basePath: tpl.dataset.basePath || "",
      emojiUrl: tpl.dataset.emojiUrl,
      labels: JSON.parse(tpl.dataset.labels || "{}"),
      icon: function (name) {
        var icon = tpl.content.querySelector(".d-icon-" + name);
        return icon ? icon.outerHTML : "";
      },
    };
  }

  function loadList(cfg) {
    if (list) {
      return Promise.resolve(list);
    }
    if (!listRequest) {
      listRequest = fetch(cfg.basePath + "/emojis.json", { credentials: "same-origin" })
        .then(function (r) {
          return r.json();
        })
        .then(function (groups) {
          list = groups;
          Object.keys(groups).forEach(function (group) {
            groups[group].forEach(function (emoji) {
              urls[emoji.name] = emoji.url;
              if (emoji.tonable) {
                tonableNames[emoji.name] = true;
              }
            });
          });
          return list;
        });
    }
    return listRequest;
  }

  function loadSearchAliases(cfg) {
    if (searchAliases) {
      return Promise.resolve(searchAliases);
    }
    return fetch(cfg.basePath + "/emojis/search-aliases.json", { credentials: "same-origin" })
      .then(function (r) {
        return r.json();
      })
      .then(function (aliases) {
        searchAliases = aliases;
        return aliases;
      });
  }

  // emojiUrlFor: a known emoji's image, toned when asked.
  function emojiUrl(cfg, code) {
    var match = /^(.+?)(?::t([1-6]))?$/.exec(code);
    var name = match[1];
    var tone = match[2];
    if (tone) {
      return cfg.emojiUrl.replace("%{name}", name + "/" + tone);
    }
    return urls[name] || cfg.emojiUrl.replace("%{name}", name);
  }

  function isTonable(code) {
    return !hasTone(code) && !!tonableNames[code.split(":").filter(Boolean)[0]];
  }

  // emojiSearch: names starting with the term, then those whose search
  // terms do, then names holding it; fifty at most.
  function emojiSearch(term, aliases) {
    if (!toSearch) {
      toSearch = Object.keys(urls).sort();
    }
    var results = [];
    var add = function (name) {
      if (results.indexOf(name) === -1 && urls[name] !== undefined) {
        results.push(name);
      }
    };
    toSearch.forEach(function (item) {
      if (item.indexOf(term) === 0) {
        add(item);
      }
    });
    Object.keys(aliases).forEach(function (key) {
      aliases[key].forEach(function (item) {
        if (item.indexOf(term) === 0) {
          add(key);
        }
      });
    });
    toSearch.forEach(function (item) {
      if (item.indexOf(term) > 0) {
        add(item);
      }
    });
    return results.slice(0, 50);
  }

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  // dReplaceEmoji's image.
  function replaceEmoji(cfg, code) {
    return (
      '<img width="20" height="20" src="' + escapeHtml(emojiUrl(cfg, code)) + '" title="' +
      escapeHtml(code) + '" alt="' + escapeHtml(code) + '" class="emoji">'
    );
  }

  function emojiImg(cfg, emoji, tabindex) {
    var tone = diversity();
    var toned = emoji.tonable && tone !== 1;
    var src = toned ? emojiUrl(cfg, emoji.name + ":t" + tone) : emojiUrl(cfg, emoji.name);
    var title = toned ? ":" + emoji.name + ":t" + tone + ":" : ":" + emoji.name + ":";
    return (
      '<img alt="' + escapeHtml(emoji.name) + '" class="emoji" data-emoji="' + escapeHtml(emoji.name) +
      '"' + (emoji.tonable ? ' data-tonable="true"' : "") + ' height="32" loading="lazy" src="' +
      escapeHtml(src) + '" tabindex="' + tabindex + '" title="' + escapeHtml(title) + '" width="32">'
    );
  }

  // The picker's groups: the context's favorites, then /emojis.json's.
  function groups(cfg, context) {
    var out = {
      favorites: favorites(context, function (emoji) {
        return !!tonableNames[emoji.split(":")[0]];
      })
        .filter(function (name) {
          return urls[name.split(":")[0]] !== undefined;
        })
        .map(function (name) {
          return { name: name, group: "favorites", url: emojiUrl(cfg, name) };
        }),
    };
    Object.keys(list).forEach(function (group) {
      out[group] = list[group];
    });
    return out;
  }

  function sectionLabel(cfg, section) {
    return cfg.labels[section] || section;
  }

  function sectionsHtml(cfg, all) {
    var out = "";
    Object.keys(all).forEach(function (section) {
      var emojis = all[section];
      if (!emojis.length) {
        return;
      }
      var label = escapeHtml(sectionLabel(cfg, section));
      out +=
        '<div aria-label="' + label + '" class="emoji-picker__section" data-section="' +
        escapeHtml(section) + '" role="region"><div class="emoji-picker__section-title-container">' +
        '<h2 class="emoji-picker__section-title">' + label + "</h2>";
      if (section === "favorites") {
        out +=
          '<button class="btn no-text btn-icon btn-transparent" type="button">' +
          cfg.icon("trash-can") + '<span aria-hidden="true">&#8203;</span></button>';
      }
      out += '</div><div class="emoji-picker__section-emojis">';
      emojis.forEach(function (emoji, index) {
        out += emojiImg(cfg, emoji, index === 0 ? 0 : -1);
      });
      out += "</div></div>";
    });
    return out;
  }

  function navHtml(cfg, all) {
    var out = "";
    var tone = diversity();
    Object.keys(all).forEach(function (section) {
      var emojis = all[section];
      if (!emojis.length) {
        return;
      }
      var icon;
      if (section === "favorites") {
        icon = replaceEmoji(cfg, "star");
      } else {
        var first = emojis[0];
        var src = first.tonable && tone !== 1 ? emojiUrl(cfg, first.name + ":t" + tone) : emojiUrl(cfg, first.name);
        icon = '<img class="emoji" height="18" src="' + escapeHtml(src) + '" width="18">';
      }
      out +=
        '<button class="btn no-text btn-flat emoji-picker__section-btn" tabindex="-1" data-section="' +
        escapeHtml(section) + '" type="button">' + icon + "</button>";
    });
    return out;
  }

  function diversityTrigger(cfg) {
    var tone = diversity();
    return replaceEmoji(cfg, tone === 1 ? "clap" : "clap:t" + tone);
  }

  var current = null;

  function portals() {
    var node = document.getElementById("d-menu-portals");
    if (!node) {
      node = document.createElement("div");
      node.id = "d-menu-portals";
      document.body.appendChild(node);
    }
    return node;
  }

  // computePosition with flip: the first placement that fits the window.
  function placementRect(placement, t, w, h) {
    var parts = placement.split("-");
    var side = parts[0];
    var align = parts[1];
    var x;
    var y;
    if (side === "top" || side === "bottom") {
      y = side === "top" ? t.top - h - MENU_OFFSET : t.bottom + MENU_OFFSET;
      x = align === "start" ? t.left : align === "end" ? t.right - w : t.left + t.width / 2 - w / 2;
    } else {
      x = side === "left" ? t.left - w - MENU_OFFSET : t.right + MENU_OFFSET;
      y = align === "start" ? t.top : align === "end" ? t.bottom - h : t.top + t.height / 2 - h / 2;
    }
    return { x: x, y: y };
  }

  function place(trigger, content, placements) {
    var t = trigger.getBoundingClientRect();
    var w = content.offsetWidth;
    var h = content.offsetHeight;
    var vw = document.documentElement.clientWidth;
    var vh = window.innerHeight;
    var chosen = placements[0];
    var spot = placementRect(chosen, t, w, h);
    for (var i = 0; i < placements.length; i++) {
      var candidate = placementRect(placements[i], t, w, h);
      if (candidate.x >= 0 && candidate.y >= 0 && candidate.x + w <= vw && candidate.y + h <= vh) {
        chosen = placements[i];
        spot = candidate;
        break;
      }
    }
    content.dataset.placement = chosen;
    content.style.left = spot.x + window.scrollX + "px";
    content.style.top = spot.y + window.scrollY + "px";
    content.style.visibility = "visible";
  }

  function menuElement(classes, identifier, maxWidth) {
    var content = document.createElement("div");
    content.className = classes;
    content.dataset.content = "";
    if (identifier) {
      content.dataset.identifier = identifier;
    }
    content.setAttribute("role", "dialog");
    content.style.maxWidth = "min(" + (maxWidth || MENU_MAX_WIDTH) + "px, -20px + 100dvw)";
    content.style.visibility = "hidden";
    content.dataset.strategy = "absolute";
    return content;
  }

  // DOverflowControls: a chevron on each edge the nav can still scroll to.
  function updateOverflow(nav) {
    var wrap = nav.parentNode;
    var cfg = config();
    var overflows = nav.scrollHeight > nav.clientHeight + 1;
    var atStart = nav.scrollTop <= 0;
    var atEnd = nav.scrollTop + nav.clientHeight >= nav.scrollHeight - 1;
    nav.dataset.dScrollAxis = "vertical";
    nav.toggleAttribute("data-d-scroll-overflow", overflows);
    nav.toggleAttribute("data-d-scroll-at-start", overflows && atStart);
    nav.toggleAttribute("data-d-scroll-at-end", overflows && atEnd);
    [
      ["up", overflows && !atStart],
      ["down", overflows && !atEnd],
    ].forEach(function (edge) {
      var button = wrap.querySelector(".d-overflow-controls__btn.--" + edge[0]);
      if (edge[1] && !button) {
        button = document.createElement("button");
        button.setAttribute("aria-hidden", "true");
        button.className = "d-overflow-controls__btn --" + edge[0];
        button.tabIndex = -1;
        button.type = "button";
        button.innerHTML = cfg.icon("chevron-" + edge[0]);
        if (edge[0] === "up") {
          wrap.insertBefore(button, nav);
        } else {
          wrap.appendChild(button);
        }
      } else if (!edge[1] && button) {
        button.remove();
      }
    });
  }

  function open(trigger, options) {
    var cfg = config();
    if (!cfg) {
      return;
    }
    if (current && current.trigger === trigger) {
      close();
      return;
    }
    close();
    var context = options.context || "topic";
    var content = menuElement("fk-d-menu -animated -expanded", "emoji-picker", options.maxWidth);
    content.innerHTML =
      '<div class="fk-d-menu__inner-content"><div class="emoji-picker">' +
      '<div class="emoji-picker__filter-container"><div class="emoji-picker__filter filter-input-container">' +
      cfg.icon("magnifying-glass") +
      '<input autocapitalize="none" autocomplete="off" autocorrect="off" class="filter-input" placeholder="' +
      escapeHtml(cfg.labels.search_placeholder || "") + '" type="text" autofocus=""></div>' +
      '<button aria-expanded="false" class="btn no-text fk-d-menu__trigger -trigger emoji-picker__diversity-trigger btn-transparent" data-trigger="" type="button">' +
      diversityTrigger(cfg) + "</button></div>" +
      '<div class="emoji-picker__content"><div class="spinner-container"><div class="spinner medium"></div></div></div>' +
      "</div></div>";
    portals().appendChild(content);
    trigger.setAttribute("aria-expanded", "true");
    var state = {
      trigger: trigger,
      content: content,
      context: context,
      onSelect: options.onSelect,
      onClose: options.onClose,
      filterTimer: null,
      diversityMenu: null,
    };
    current = state;
    place(trigger, content, PLACEMENTS);
    var input = content.querySelector(".filter-input");
    input.value = options.term || "";
    input.focus({ preventScroll: true });
    loadList(cfg).then(function () {
      if (current !== state) {
        return;
      }
      render(cfg, state);
      filter(cfg, state, input.value);
      place(trigger, content, PLACEMENTS);
      updateOverflow(content.querySelector(".emoji-picker__sections-nav"));
    });
  }

  function render(cfg, state) {
    var all = groups(cfg, state.context);
    var body = state.content.querySelector(".emoji-picker__content");
    body.innerHTML =
      '<div class="d-overflow-controls emoji-picker__sections-nav-wrap">' +
      '<div class="d-overflow-controls__content emoji-picker__sections-nav">' + navHtml(cfg, all) + "</div></div>" +
      '<div class="emoji-picker__scrollable-content"><div class="emoji-picker__sections" role="button">' +
      sectionsHtml(cfg, all) + "</div></div>";
    var nav = body.querySelector(".emoji-picker__sections-nav");
    // lastVisibleSection starts at the favorites.
    var favoritesButton = nav.querySelector('.emoji-picker__section-btn[data-section="favorites"]');
    if (favoritesButton) {
      favoritesButton.classList.add("active");
    }
    nav.addEventListener("scroll", function () {
      updateOverflow(nav);
    });
    updateOverflow(nav);
    body.querySelector(".emoji-picker__scrollable-content").addEventListener("scroll", function (event) {
      onScroll(state, event.target);
    });
  }

  // _handleScroll: the nav follows the section in view.
  function onScroll(state, scroller) {
    if (state.scrolling) {
      return;
    }
    var box = scroller.getBoundingClientRect();
    var down = scroller.scrollTop > (state.prevY || 0);
    var visible = Array.prototype.filter.call(
      scroller.querySelectorAll(".emoji-picker__section:not(.filtered)"),
      function (section) {
        var r = section.getBoundingClientRect();
        return r.top <= box.top ? box.top - r.top <= r.height : r.bottom - box.bottom <= r.height;
      }
    );
    var y = scroller.scrollTop;
    if (visible.length) {
      var section = !down || (state.prevY || 0) < 50 ? visible[0] : visible[visible.length - 1];
      setActive(state, section.dataset.section);
    }
    state.prevY = y;
  }

  function setActive(state, section) {
    state.content.querySelectorAll(".emoji-picker__section-btn").forEach(function (button) {
      var active = button.dataset.section === section;
      button.classList.toggle("active", active);
      if (active) {
        button.scrollIntoView({ block: "nearest", inline: "start" });
      }
    });
  }

  // didInputFilter / debouncedDidInputFilter
  function filter(cfg, state, value) {
    clearTimeout(state.filterTimer);
    var sections = state.content.querySelector(".emoji-picker__sections");
    if (!sections) {
      return;
    }
    var filtered = sections.querySelector(".emoji-picker__section.filtered");
    if (!value) {
      if (filtered) {
        filtered.remove();
      }
      sections.querySelectorAll(".emoji-picker__section").forEach(function (section) {
        section.classList.remove("hidden");
      });
      return;
    }
    if (!filtered) {
      filtered = document.createElement("div");
      filtered.className = "emoji-picker__section filtered";
      filtered.innerHTML = '<div class="spinner-container"><div class="spinner medium"></div></div>';
      sections.querySelectorAll(".emoji-picker__section").forEach(function (section) {
        section.classList.add("hidden");
      });
      sections.insertBefore(filtered, sections.firstChild);
    }
    state.filterTimer = setTimeout(function () {
      loadSearchAliases(cfg).then(function (aliases) {
        if (current !== state || !filtered.isConnected) {
          return;
        }
        var results = emojiSearch(value.toLowerCase(), aliases);
        if (results.length) {
          filtered.innerHTML = results
            .map(function (name, index) {
              return emojiImg(cfg, { name: name, tonable: isTonable(name) }, index === 0 ? 0 : -1);
            })
            .join("");
        } else {
          filtered.innerHTML =
            '<p class="emoji-picker__no-results">' + escapeHtml(cfg.labels.no_results || "") + " " +
            replaceEmoji(cfg, "crying_cat_face") + "</p>";
        }
        var scroller = state.content.querySelector(".emoji-picker__scrollable-content");
        if (scroller) {
          scroller.scrollTop = 0;
        }
      });
    }, INPUT_DELAY);
  }

  // didSelectEmoji: the emoji, toned when it can be, counted as used.
  function select(state, img) {
    var emoji = img.dataset.emoji;
    var tone = diversity();
    if (img.dataset.tonable && tone > 1) {
      emoji = emoji + ":t" + tone;
    }
    track(emoji, state.context);
    var onSelect = state.onSelect;
    close();
    if (onSelect) {
      onSelect(emoji);
    }
  }

  // didRequestSection: the filter cleared, the section scrolled to.
  function requestSection(cfg, state, section) {
    var input = state.content.querySelector(".filter-input");
    input.value = "";
    filter(cfg, state, "");
    var target = state.content.querySelector('.emoji-picker__section[data-section="' + section + '"]');
    var scroller = state.content.querySelector(".emoji-picker__scrollable-content");
    if (!target || !scroller) {
      return;
    }
    state.scrolling = true;
    var title = target.querySelector(".emoji-picker__section-title-container");
    var top = target.offsetTop - (title ? title.offsetHeight : 0);
    scroller.scrollTop = Math.min(top, scroller.scrollHeight - scroller.clientHeight);
    setActive(state, section);
    setTimeout(function () {
      state.scrolling = false;
    }, 50);
  }

  function openDiversityMenu(cfg, state, trigger) {
    if (state.diversityMenu) {
      closeDiversityMenu(state);
      return;
    }
    var menu = menuElement("fk-d-menu -content emoji-picker__diversity-menu -animated -expanded", null);
    var items = "";
    for (var scale = 1; scale <= 6; scale++) {
      items +=
        '<li class="dropdown-menu__item"><button class="btn no-text btn-transparent emoji-picker__diversity-item"' +
        (scale > 1 ? ' data-level="' + scale + '"' : "") + ' type="button">' +
        replaceEmoji(cfg, scale > 1 ? "clap:t" + scale : "clap") + "</button></li>";
    }
    menu.innerHTML = '<div class="fk-d-menu__inner-content"><ul class="dropdown-menu">' + items + "</ul></div>";
    portals().appendChild(menu);
    trigger.setAttribute("aria-expanded", "true");
    state.diversityMenu = menu;
    place(trigger, menu, PLACEMENTS);
  }

  function closeDiversityMenu(state) {
    if (state.diversityMenu) {
      state.diversityMenu.remove();
      state.diversityMenu = null;
      var trigger = state.content.querySelector(".emoji-picker__diversity-trigger");
      if (trigger) {
        trigger.setAttribute("aria-expanded", "false");
      }
    }
  }

  // didRequestFitzpatrickScale: the tone kept, the picker drawn with it.
  function setDiversity(cfg, state, scale) {
    storeSet("emojiSelectedDiversity", scale);
    closeDiversityMenu(state);
    state.content.querySelector(".emoji-picker__diversity-trigger").innerHTML = diversityTrigger(cfg);
    var input = state.content.querySelector(".filter-input");
    render(cfg, state);
    filter(cfg, state, input.value);
  }

  function close() {
    var state = current;
    if (!state) {
      return;
    }
    current = null;
    clearTimeout(state.filterTimer);
    closeDiversityMenu(state);
    state.content.remove();
    state.trigger.setAttribute("aria-expanded", "false");
    if (state.onClose) {
      state.onClose();
    }
  }

  // didNavigateSection: the arrows move between emoji, out to the filter.
  function navigate(state, event) {
    var target = event.target;
    var section = target.closest(".emoji-picker__section");
    var input = state.content.querySelector(".filter-input");
    var emojis = function (node) {
      return Array.prototype.slice.call(node.querySelectorAll(".emoji"));
    };
    var all = function () {
      return Array.prototype.slice.call(
        state.content.querySelectorAll(".emoji-picker__section:not(.hidden) .emoji")
      );
    };
    if (event.key === "ArrowRight") {
      event.preventDefault();
      var next = target.nextElementSibling;
      if (next) {
        next.focus();
      } else if (section.nextElementSibling) {
        emojis(section.nextElementSibling)[0].focus();
      }
    } else if (event.key === "ArrowLeft") {
      event.preventDefault();
      var prev = target.previousElementSibling;
      if (prev) {
        prev.focus();
      } else if (section.previousElementSibling) {
        var list = emojis(section.previousElementSibling);
        list[list.length - 1].focus();
      } else {
        input.focus();
      }
    } else if (event.key === "ArrowDown") {
      event.preventDefault();
      event.stopPropagation();
      var below = all()
        .filter(function (c) {
          return c.offsetTop > target.offsetTop;
        })
        .find(function (c) {
          return c.offsetLeft === target.offsetLeft;
        });
      if (below) {
        below.focus();
      } else if (section.nextElementSibling) {
        emojis(section.nextElementSibling)[0].focus();
      }
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      event.stopPropagation();
      var above = all()
        .reverse()
        .filter(function (c) {
          return c.offsetTop < target.offsetTop;
        })
        .find(function (c) {
          return c.offsetLeft === target.offsetLeft;
        });
      if (above) {
        above.focus();
      } else {
        input.focus();
      }
    }
  }

  document.addEventListener("input", function (event) {
    if (current && event.target.closest(".emoji-picker__filter")) {
      filter(config(), current, event.target.value);
    }
  });

  document.addEventListener("click", function (event) {
    var state = current;
    if (!state) {
      return;
    }
    var cfg = config();
    var target = event.target;
    if (state.diversityMenu && state.diversityMenu.contains(target)) {
      var item = target.closest(".emoji-picker__diversity-item");
      if (item) {
        setDiversity(cfg, state, Number(item.dataset.level || 1));
      }
      return;
    }
    if (!state.content.contains(target)) {
      // closeOnClickOutside; a click on the trigger toggles it.
      if (!state.trigger.contains(target)) {
        close();
      }
      return;
    }
    closeDiversityMenu(state);
    var emoji = target.closest(".emoji-picker__sections .emoji");
    if (emoji) {
      event.preventDefault();
      event.stopPropagation();
      select(state, emoji);
      return;
    }
    var sectionButton = target.closest(".emoji-picker__section-btn");
    if (sectionButton) {
      requestSection(cfg, state, sectionButton.dataset.section);
      return;
    }
    var diversityButton = target.closest(".emoji-picker__diversity-trigger");
    if (diversityButton) {
      openDiversityMenu(cfg, state, diversityButton);
      return;
    }
    if (target.closest(".emoji-picker__section[data-section='favorites'] .emoji-picker__section-title-container .btn")) {
      resetContext(state.context);
      render(cfg, state);
      return;
    }
    var chevron = target.closest(".d-overflow-controls__btn");
    if (chevron) {
      var nav = state.content.querySelector(".emoji-picker__sections-nav");
      var by = chevron.classList.contains("--up") ? -nav.clientHeight : nav.clientHeight;
      nav.scrollBy({ top: by, behavior: "smooth" });
    }
  }, true);

  document.addEventListener("keydown", function (event) {
    var state = current;
    if (!state) {
      return;
    }
    if (event.key === "Escape") {
      // closeOnEscape: the skin tone menu first, then the picker.
      event.stopPropagation();
      if (state.diversityMenu) {
        closeDiversityMenu(state);
      } else {
        var trigger = state.trigger;
        close();
        trigger.focus({ preventScroll: true });
      }
      return;
    }
    if (!state.content.contains(event.target)) {
      return;
    }
    // trapKeyDownEvents
    if (event.key === "ArrowUp") {
      event.stopPropagation();
    }
    if (event.key === "ArrowDown" && event.target.classList.contains("filter-input")) {
      event.stopPropagation();
      event.preventDefault();
      var first = state.content.querySelector('.emoji-picker__sections .emoji[tabindex="0"]');
      if (first) {
        first.focus();
      }
      return;
    }
    if (event.target.matches(".emoji-picker__sections .emoji")) {
      if (event.key === "Enter") {
        event.preventDefault();
        event.stopPropagation();
        select(state, event.target);
      } else {
        navigate(state, event);
      }
    }
  }, true);

  window.addEventListener("resize", function () {
    if (current) {
      place(current.trigger, current.content, PLACEMENTS);
    }
  });


  var clientData = null;

  function loadClientData(cfg) {
    if (!clientData) {
      clientData = fetch(cfg.basePath + "/assets/emoji-data.json", { credentials: "same-origin" })
        .then(function (r) {
          return r.json();
        })
        .then(function (data) {
          data.nameSet = {};
          data.names.forEach(function (name) {
            data.nameSet[name] = true;
          });
          return data;
        });
    }
    return clientData;
  }

  // reactingToLastMessage's emoji: "+" and a unicode emoji, else "+:code:"
  // (its inner part, as substring takes it) normalized (normalizeEmoji: a
  // standard or custom name, else an alias's name). Null when it names
  // none.
  function shortcutReaction(text) {
    var cfg = config();
    if (!cfg || text.charAt(0) !== "+") {
      return Promise.resolve(null);
    }
    return Promise.all([loadClientData(cfg), loadList(cfg)]).then(function (loaded) {
      var data = loaded[0];
      var reaction = text.substring(1);
      if (data.unicode[reaction]) {
        return data.unicode[reaction];
      }
      var code = reaction.substring(1, reaction.length - 1).toLowerCase();
      if (urls[code] !== undefined || data.nameSet[code]) {
        return code;
      }
      return data.aliases[code] || null;
    });
  }

  Discourse.emojiPicker = {
    open: open,
    shortcutReaction: shortcutReaction,
    close: close,
    isOpen: function () {
      return !!current;
    },
  };
})();
