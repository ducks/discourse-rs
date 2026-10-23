// A message's AddPmParticipants modal (components/modal/
// add-pm-participants.gjs with InvitePanel), which the topic map's add
// participant button opens: an EmailGroupUserChooser (maximum 1, emails
// when the viewer may invite by email, the messageable groups, without
// the viewer) over /u/search/users, ranked as lib/user-search's
// organizeResults ranks them. Send Invite POSTs /t/:id/invite (a user or
// an email, with the custom message an email may carry) or
// /t/:id/invite-group, shows the server's error in the panel or, done,
// closes with a success toast and draws the topic map's participants
// again from the server, where Ember adds the user to its own list.
// The page's #add-pm-participants-config holds the strings and options.
(function () {
  "use strict";

  if (!Discourse.once("add-pm-participants")) {
    return;
  }

  function config() {
    var el = document.getElementById("add-pm-participants-config");
    try {
      return el ? JSON.parse(el.textContent) : null;
    } catch (e) {
      return null;
    }
  }

  function base() {
    return (document.querySelector('meta[name="discourse-base-uri"]') || {}).content || "";
  }

  function memberHeaders() {
    try {
      return JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (e) {
      return {};
    }
  }

  function escapeHtml(s) {
    return String(s == null ? "" : s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function icon(name, extra) {
    return (
      '<svg class="fa d-icon d-icon-' +
      name +
      " svg-icon fa-width-auto" +
      (extra ? " " + extra : "") +
      ' svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name +
      '"></use></svg>'
    );
  }

  // lib/utilities' emailValid
  var EMAIL =
    /^[a-zA-Z0-9!#$%&'*+\/=?\^_`{|}~\-]+(?:\.[a-zA-Z0-9!#$%&'\*+\/=?\^_`{|}~\-]+)*@(?:[a-zA-Z0-9](?:[a-zA-Z0-9\-]*[a-zA-Z0-9])?\.)+[a-zA-Z0-9](?:[a-zA-Z0-9\-]*[a-zA-Z0-9])?$/;
  function emailValid(s) {
    return EMAIL.test(s);
  }

  // lib/user-search's skipSearch
  var IGNORE = /([ -⁯⸀-⹿\\'!"#$%&()*,\/:;<=>?\[\]^`{|}~])|\s\s|^\s$|^[^+]*\+[^@]*$/;
  function skipSearch(term, allowEmails) {
    if (term.indexOf("@") !== -1 && !allowEmails) {
      return true;
    }
    return IGNORE.test(term);
  }

  function lowerCaseIncludes(string, term) {
    return !!(string && term && string.toLowerCase().indexOf(term.toLowerCase()) !== -1);
  }

  // lib/user-search's organizeResults, then EmailGroupUserChooser#search's
  // items: { id, name, isUser | isGroup | isEmail, avatar_template }.
  function organize(r, term, exclude, allowEmails, limit) {
    var first = ((term || "").trim().split(/\s/, 1) || [])[0];
    var users = (r.users || []).filter(function (u) {
      return exclude.indexOf(u.username) === -1;
    });
    users.forEach(function (u) {
      u.isMetadataMatch = !!first && !lowerCaseIncludes(u.username, first) && !lowerCaseIncludes(u.name, first);
    });
    var groups = (r.groups || []).filter(function (g) {
      return exclude.indexOf(g.name) === -1;
    });
    var emails = allowEmails && emailValid(term) ? [{ username: term, isEmail: true }] : [];
    var lower = function (v) {
      return v == null ? v : String(v).toLowerCase();
    };
    var seenUsers = {};
    var seenGroups = {};
    var pickUsers = function (test) {
      return users.filter(function (u) {
        if (seenUsers[u.username] || !test(u)) {
          return false;
        }
        seenUsers[u.username] = true;
        return true;
      });
    };
    var pickGroups = function (test) {
      return groups.filter(function (g) {
        if (seenGroups[g.name] || !test(g)) {
          return false;
        }
        seenGroups[g.name] = true;
        return true;
      });
    };
    var ranked = [].concat(
      pickUsers(function (u) {
        return lower(u.username) === lower(first);
      }),
      pickGroups(function (g) {
        return lower(g.name) === lower(first);
      }),
      emails,
      pickUsers(function (u) {
        return !u.isMetadataMatch && !(first && !lowerCaseIncludes(u.username, first));
      }),
      pickGroups(function (g) {
        return !(first && !lowerCaseIncludes(g.name, first));
      }),
      pickUsers(function (u) {
        return lower(u.name) === lower(first) && !u.isMetadataMatch;
      }),
      pickUsers(function (u) {
        return lowerCaseIncludes(u.name, first) && !u.isMetadataMatch;
      }),
      pickGroups(function (g) {
        return !!g.full_name && lower(g.full_name) === lower(first);
      }),
      pickGroups(function (g) {
        return !!g.full_name && lowerCaseIncludes(g.full_name, first);
      }),
      users.filter(function (u) {
        return u.isMetadataMatch;
      })
    );
    return ranked.slice(0, limit).map(function (item) {
      if (item.isEmail) {
        return { id: item.username, isEmail: true };
      }
      if (item.username) {
        return { id: item.username, name: item.name, isUser: true, avatar_template: item.avatar_template };
      }
      return { id: item.name, name: item.full_name || item.name, full_name: item.full_name, isGroup: true };
    });
  }

  function avatar(template, username) {
    var root = base();
    var src = String(template || "").replace("{size}", "24");
    if (root && src.indexOf(root) !== 0 && src.charAt(0) === "/" && src.charAt(1) !== "/") {
      src = root + src;
    }
    return (
      '<img alt="" width="24" height="24" src="' +
      escapeHtml(src) +
      '" class="avatar" title="' +
      escapeHtml(username) +
      '">'
    );
  }

  // EmailGroupUserChooserRow, in a select-kit row.
  function row(item, index) {
    var attrs =
      'aria-checked="false" role="menuitemradio" data-index="' +
      index +
      '"' +
      (item.name ? ' data-name="' + escapeHtml(item.name) + '"' : "") +
      ' data-value="' +
      escapeHtml(item.id) +
      '"' +
      (item.name ? ' title="' + escapeHtml(item.name) + '"' : "") +
      ' tabindex="0" class="select-kit-row email-group-user-chooser-row' +
      (index === 0 ? " is-highlighted" : "") +
      '"';
    var inner;
    if (item.isUser) {
      inner =
        avatar(item.avatar_template, item.id) +
        '<div class="email-group-user-chooser__user --username-first"><span class="identifier">' +
        escapeHtml(item.id) +
        "</span>" +
        (item.name ? ' <span class="name">' + escapeHtml(item.name) + "</span>" : "") +
        "</div>";
    } else if (item.isGroup) {
      inner =
        icon("users") +
        '<div class="email-group-user-chooser__group --group-name-first"><span class="identifier">' +
        escapeHtml(item.id) +
        '</span> <span class="name">' +
        escapeHtml(item.full_name || "") +
        "</span></div>";
    } else {
      inner = icon("envelope") + '<span class="identifier">' + escapeHtml(item.id) + "</span>";
    }
    return "<li " + attrs + ">" + inner + "</li>";
  }

  function show(cfg) {
    var s = cfg.strings;
    var state = {
      invitee: null,
      isGroup: false,
      saving: false,
      hasCustomMessage: false,
      customMessage: null,
    };
    var panel;
    var chooser;
    var searchTimer = null;
    var searchSeq = 0;
    var shown = [];

    // select-kit's classes, its state ones before full-width-wrap.
    function classes() {
      chooser.className =
        "select-kit multi-select user-chooser email-group-user-chooser" +
        (state.invitee ? " has-selection" : "") +
        (chooser.open ? " is-expanded" : "") +
        " full-width-wrap invite-user-input";
    }

    function header() {
      var summary = chooser.querySelector("summary");
      var wrapper = summary.querySelector(".select-kit-header-wrapper");
      if (state.invitee) {
        summary.dataset.name = state.invitee;
        summary.dataset.value = state.invitee;
        wrapper.innerHTML = '<span class="formatted-selection">' + escapeHtml(state.invitee) + "</span>";
      } else {
        summary.dataset.name = "";
        delete summary.dataset.value;
        wrapper.innerHTML =
          '<span class="formatted-selection">' + escapeHtml(s.defaultHeader) + "</span>" + icon("plus", "angle-icon");
      }
      classes();
    }

    // InvitePanel#disabled and #showCustomMessage, the footer's button.
    function sync() {
      var send = panel.querySelector(".send-invite");
      send.disabled = state.saving || !state.invitee;
      send.querySelector(".d-button-label").textContent = state.saving ? s.inviting : s.send;
      var custom = panel.querySelector(".show-custom-message-control");
      var showCustom = !!state.invitee && emailValid(state.invitee);
      if (showCustom && !custom) {
        custom = document.createElement("div");
        custom.className = "show-custom-message-control";
        custom.innerHTML = '<label class="instructions"><span class="optional">' + s.customMessage + "</span></label>";
        panel.querySelector(".invite-user-control").after(custom);
      } else if (!showCustom && custom) {
        custom.remove();
        state.hasCustomMessage = false;
        state.customMessage = null;
      }
    }

    function position() {
      var body = chooser.querySelector(".select-kit-body");
      var summary = chooser.querySelector("summary");
      var ui = window.FloatingUIDOM;
      if (!ui) {
        return;
      }
      body.style.position = "fixed";
      body.style.minWidth = Math.round(summary.getBoundingClientRect().width) + "px";
      body.style.top = "0px";
      body.style.left = "0px";
      ui.computePosition(summary, body, {
        placement: "bottom-start",
        strategy: "fixed",
        middleware: [ui.flip({ padding: 3 }), ui.shift({ limiter: ui.limitShift() }), ui.offset(3)],
      }).then(function (result) {
        body.style.transform = "translate(" + Math.round(result.x) + "px, " + Math.round(result.y) + "px)";
        body.style.visibility = "visible";
        body.style.pointerEvents = "auto";
      });
    }

    function renderBody(rows) {
      var body = chooser.querySelector(".select-kit-body");
      var input = body.querySelector(".filter-input");
      var term = input ? input.value : "";
      var html =
        '<div class="select-kit-filter multi-select-filter email-group-user-chooser-filter is-expanded"><input class="ember-text-field filter-input" autocapitalize="off" autocomplete="off" autocorrect="off" name="filter-input-search" placeholder="' +
        escapeHtml(s.placeholder) +
        '" spellcheck="false" tabindex="0" type="search"></div>';
      shown = rows || [];
      if (rows && rows.length) {
        html +=
          '<ul aria-live="polite" class="select-kit-collection" role="menu">' +
          rows.map(row).join("") +
          '</ul><span class="results-count" role="alert">' +
          escapeHtml((rows.length === 1 ? s.resultsOne : s.resultsOther).replace("%{count}", rows.length)) +
          "</span>";
      }
      body.innerHTML = html;
      var fresh = body.querySelector(".filter-input");
      fresh.value = term;
      fresh.focus();
      position();
    }

    // UserChooser#search through lib/user-search, debounced.
    function search() {
      var input = chooser.querySelector(".select-kit-body .filter-input");
      var term = (input ? input.value : "").replace(/^@/, "");
      clearTimeout(searchTimer);
      var seq = ++searchSeq;
      if (term === "" || skipSearch(term, cfg.canInviteViaEmail)) {
        renderBody(null);
        return;
      }
      searchTimer = setTimeout(function () {
        var params = new URLSearchParams({
          term: term,
          include_groups: "false",
          include_mentionable_groups: "false",
          include_messageable_groups: "true",
          topic_allowed_users: "false",
          limit: "6",
        });
        var exclude = state.invitee ? [state.invitee] : [];
        exclude.push(cfg.currentUsername);
        fetch(base() + "/u/search/users.json?" + params.toString(), {
          credentials: "same-origin",
          headers: { "X-Requested-With": "XMLHttpRequest", Accept: "application/json" },
        })
          .then(function (r) {
            return r.ok ? r.json() : {};
          })
          .then(function (json) {
            if (seq !== searchSeq || !chooser.open) {
              return;
            }
            renderBody(organize(json, term, exclude, cfg.canInviteViaEmail, 6));
          })
          .catch(function () {});
      }, 300);
    }

    function choose(li) {
      state.invitee = li.dataset.value.trim();
      var item = shown[parseInt(li.dataset.index, 10)];
      state.isGroup = !!(item && item.isGroup);
      chooser.open = false;
      header();
      sync();
    }

    function showError(message) {
      var alert = panel.querySelector(".alert.alert-error");
      if (!alert) {
        alert = document.createElement("div");
        alert.className = "alert alert-error";
        panel.insertBefore(alert, panel.firstChild);
      }
      // trustHTML: the server's error is HTML.
      alert.innerHTML = message;
    }

    // The topic map's participants as the server now draws them.
    function redrawParticipants() {
      return fetch(location.href, { credentials: "same-origin" })
        .then(function (r) {
          return r.text();
        })
        .then(function (html) {
          var doc = new DOMParser().parseFromString(html, "text/html");
          var fresh = doc.querySelectorAll(".topic-map .participants");
          document.querySelectorAll(".topic-map .participants").forEach(function (el, i) {
            if (fresh[i]) {
              el.replaceWith(document.importNode(fresh[i], true));
            }
          });
        })
        .catch(function () {});
    }

    function toast(text) {
      var section = document.querySelector("section.fk-d-toasts");
      if (!section) {
        section = document.createElement("section");
        section.className = "fk-d-toasts";
        document.body.appendChild(section);
      }
      var output = document.createElement("output");
      output.className = "fk-d-toast";
      output.setAttribute("role", "status");
      output.innerHTML =
        '<div class="fk-d-default-toast -success"><div class="fk-d-default-toast__icon-container">' +
        icon("check") +
        '</div><div class="fk-d-default-toast__main-container"><div class="fk-d-default-toast__texts"><div class="fk-d-default-toast__message"></div></div></div><div class="fk-d-default-toast__close-container"><button class="btn no-text btn-icon btn-transparent" type="button">' +
        icon("xmark") +
        '<span aria-hidden="true">&#8203;</span></button></div></div>';
      output.querySelector(".fk-d-default-toast__message").textContent = text;
      section.appendChild(output);
      var remove = function () {
        output.remove();
      };
      output.querySelector("button").addEventListener("click", remove);
      setTimeout(remove, 3000);
    }

    // InvitePanel#createInvite
    function createInvite(close) {
      if (state.saving || !state.invitee) {
        return;
      }
      state.saving = true;
      sync();
      var form = new URLSearchParams();
      var path;
      if (state.isGroup) {
        path = "/t/" + cfg.topicId + "/invite-group";
        form.set("group", state.invitee.trim());
      } else {
        path = "/t/" + cfg.topicId + "/invite";
        form.set("user", state.invitee.trim());
        if (state.customMessage != null) {
          form.set("custom_message", state.customMessage);
        }
      }
      var headers = memberHeaders();
      headers["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8";
      fetch(base() + path, { method: "POST", headers: headers, credentials: "same-origin", body: form.toString() })
        .then(function (r) {
          return r
            .json()
            .catch(function () {
              return {};
            })
            .then(function (body) {
              if (!r.ok) {
                state.saving = false;
                sync();
                showError((body.errors && body.errors[0]) || s.error);
                return;
              }
              var message = state.isGroup ? s.successGroup : s.success;
              redrawParticipants().then(function () {
                toast(message);
                close();
              });
            });
        })
        .catch(function () {
          state.saving = false;
          sync();
          showError(s.error);
        });
    }

    Discourse.modal.show({
      className: "add-pm-participants",
      title: s.title,
      body: function (body, close) {
        body.classList.add("invite", "modal-panel");
        panel = document.createElement("div");
        panel.innerHTML =
          '<div class="body"><div class="invite-user-control"><label class="instructions"></label><div class="invite-user-input-wrapper"><details class="select-kit multi-select user-chooser email-group-user-chooser full-width-wrap invite-user-input"><summary data-name="" tabindex="0" class="select-kit-header multi-select-header"><div class="select-kit-header-wrapper"></div></summary><div class="select-kit-body"></div></details></div></div>' +
          (cfg.approvalNotice ? '<label class="instructions approval-notice"></label>' : "") +
          '</div><div class="footer"><button class="btn btn-icon-text btn-primary send-invite" type="button">' +
          icon("envelope") +
          '<span class="d-button-label"></span></button></div>';
        panel.querySelector(".invite-user-control .instructions").textContent = s.instructions;
        if (cfg.approvalNotice) {
          panel.querySelector(".approval-notice").textContent = s.approval;
        }
        body.appendChild(panel);
        chooser = panel.querySelector("details.email-group-user-chooser");
        header();
        sync();

        chooser.addEventListener("toggle", function () {
          classes();
          var kitBody = chooser.querySelector(".select-kit-body");
          if (!chooser.open) {
            kitBody.innerHTML = "";
            kitBody.removeAttribute("style");
            return;
          }
          renderBody(null);
        });
        chooser.addEventListener("input", function (event) {
          if (event.target.classList.contains("filter-input")) {
            search();
          }
        });
        chooser.addEventListener("keydown", function (event) {
          if (!event.target.classList.contains("filter-input")) {
            return;
          }
          if (event.key === "Enter") {
            event.preventDefault();
            var li = chooser.querySelector(".select-kit-row.is-highlighted") || chooser.querySelector(".select-kit-row");
            if (li) {
              choose(li);
            }
          } else if (event.key === "Escape") {
            event.stopPropagation();
            chooser.open = false;
          }
        });
        panel.addEventListener("click", function (event) {
          var li = event.target.closest(".select-kit-row");
          if (li && chooser.contains(li)) {
            event.preventDefault();
            choose(li);
            return;
          }
          if (chooser.open && !chooser.contains(event.target)) {
            chooser.open = false;
          }
          // showCustomMessageBox
          var link = event.target.closest(".show-custom-message-control a");
          if (link) {
            event.preventDefault();
            var control = link.closest(".show-custom-message-control");
            state.hasCustomMessage = !state.hasCustomMessage;
            var area = control.querySelector("textarea");
            if (state.hasCustomMessage) {
              state.customMessage = s.customMessageTemplate;
              area = document.createElement("textarea");
              area.placeholder = s.customMessagePlaceholder;
              area.value = state.customMessage;
              area.addEventListener("input", function () {
                state.customMessage = area.value;
              });
              control.appendChild(area);
            } else {
              state.customMessage = null;
              if (area) {
                area.remove();
              }
            }
            return;
          }
          if (event.target.closest(".send-invite")) {
            createInvite(close);
          }
        });
      },
    });
  }

  document.addEventListener("click", function (event) {
    var button = event.target.closest(".topic-map .add-participant-btn");
    if (!button || !window.Discourse || !Discourse.modal) {
      return;
    }
    var cfg = config();
    if (cfg) {
      show(cfg);
    }
  });
})();
