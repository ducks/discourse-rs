// discourse-assign on the topic page: the footer's Assign button and
// reassign menu (TopicLevelAssignMenu), the post menu's assign button
// (AssignButton) and a post's assignment menu (AssignedToPost) assign
// and unassign through AssignController, the assign modal (AssignUserForm)
// offering /assign/suggestions, then the user search as one types. Once the
// assignment changed, the topic's title, footer buttons and the drawn posts
// are redrawn in place from the topic page as it is now, as Ember's
// /staff/topic-assignment subscriber updates the topic and its posts.
(function () {
  "use strict";

  if (!Discourse.once("discourse-assign")) {
    return;
  }

  function csrfHeaders() {
    var headers = {};
    try {
      headers = JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (e) {
      // No token: the server refuses the request and says so.
    }
    headers["Content-Type"] = "application/x-www-form-urlencoded";
    headers["X-Requested-With"] = "XMLHttpRequest";
    return headers;
  }

  function basePath() {
    var url = topicUrl();
    var at = url.indexOf("/t/");
    return at > 0 ? url.slice(0, at) : "";
  }

  // popupAjaxError: the response's error or errors.
  function showError(r) {
    return r
      .json()
      .then(function (body) {
        window.alert(body.error || (body.errors || [r.statusText]).join("\n"));
      })
      .catch(function () {
        window.alert(r.statusText);
      });
  }

  function topicUrl() {
    var timeline = document.querySelector(".timeline-container[data-topic-url]");
    return timeline ? timeline.dataset.topicUrl : "";
  }

  // The topic page around the target (a post's page, else the first), its
  // title, footer buttons and the posts this page draws swapped out of band
  // as the live stream swaps a redrawn post, so the scripts hooked on
  // htmx:oobAfterSwap run as for any redraw.
  function redraw(targetType, targetId) {
    var url = topicUrl();
    var post =
      targetType === "Post" ? document.querySelector('#posts [data-post-id="' + targetId + '"]') : null;
    var wrapper = post ? post.closest("#posts > [data-post-number]") : null;
    if (wrapper) {
      url += "/" + wrapper.dataset.postNumber;
    }
    return fetch(url, { credentials: "same-origin" })
      .then(function (r) {
        if (!r.ok) {
          throw new Error(r.statusText);
        }
        return r.text();
      })
      .then(function (html) {
        var doc = new DOMParser().parseFromString(html, "text/html");
        var out = [];
        ["topic-title", "topic-footer-buttons"].forEach(function (id) {
          var now = doc.getElementById(id);
          if (now && document.getElementById(id)) {
            now.setAttribute("hx-swap-oob", "true");
            out.push(now.outerHTML);
          }
        });
        document.querySelectorAll("#posts > [data-post-number]").forEach(function (shown) {
          var number = shown.dataset.postNumber;
          var now = doc.querySelector('#posts > [data-post-number="' + number + '"]');
          if (now) {
            now.setAttribute("hx-swap-oob", "outerHTML:#posts > [data-post-number='" + number + "']");
            out.push(now.outerHTML);
          }
        });
        window.htmx.swap(document.body, out.join(""), { swapStyle: "none" });
      })
      .catch(function (e) {
        // The page can't be redrawn from here: drawn again whole.
        console.error("discourse-assign: redrawing the topic:", e.message);
        window.location.reload();
      });
  }

  function put(path, params) {
    return fetch(basePath() + path, {
      method: "PUT",
      credentials: "same-origin",
      headers: csrfHeaders(),
      body: new URLSearchParams(params).toString(),
    });
  }

  // TaskActions#unassign
  function unassign(targetType, targetId) {
    put("/assign/unassign", { target_type: targetType, target_id: targetId }).then(
      function (r) {
        if (r.ok) {
          redraw(targetType, targetId);
        } else {
          showError(r);
        }
      }
    );
  }

  // TaskActions#reassignUserToTopic
  function assignTo(targetType, targetId, username) {
    put("/assign/assign", {
      target_type: targetType,
      target_id: targetId,
      username: username,
    }).then(function (r) {
      if (r.ok) {
        redraw(targetType, targetId);
      } else {
        showError(r);
      }
    });
  }

  var modal = null;

  function closeModal() {
    if (modal) {
      modal.remove();
      modal = null;
    }
  }

  // The suggestions and search results as the chooser's options; a group
  // is told apart by its value's leading "@group:".
  function fill(list, users, groups) {
    list.innerHTML = "";
    (users || []).forEach(function (u) {
      var option = document.createElement("option");
      option.value = u.username;
      option.label = u.name ? u.username + " (" + u.name + ")" : u.username;
      list.appendChild(option);
    });
    (groups || []).forEach(function (g) {
      var option = document.createElement("option");
      option.value = g.name;
      option.label = g.full_name ? g.name + " (" + g.full_name + ")" : g.name;
      option.dataset.group = "true";
      list.appendChild(option);
    });
  }

  // The suggestions, then as one types the user search among the groups
  // whose members may be assigned (assign_allowed_on_groups), the groups
  // that may be (assign_allowed_for_groups) filtered by the term.
  var allowed = { on: [], groups: [] };
  function query(targetType, targetId, term) {
    var headers = { "X-Requested-With": "XMLHttpRequest" };
    if (!term) {
      var params = new URLSearchParams({ target_type: targetType, target_id: targetId });
      return fetch(basePath() + "/assign/suggestions.json?" + params.toString(), {
        credentials: "same-origin",
        headers: headers,
      })
        .then(function (r) {
          return r.ok ? r.json() : {};
        })
        .then(function (body) {
          allowed.on = body.assign_allowed_on_groups || [];
          allowed.groups = body.assign_allowed_for_groups || [];
          return {
            suggestions: body.suggestions,
            groups: allowed.groups.map(function (name) {
              return { name: name };
            }),
          };
        });
    }
    var search = new URLSearchParams({ term: term, include_groups: "false" });
    if (allowed.on.length) {
      search.set("groups", allowed.on.join(","));
    }
    return fetch(basePath() + "/u/search/users.json?" + search.toString(), {
      credentials: "same-origin",
      headers: headers,
    })
      .then(function (r) {
        return r.ok ? r.json() : {};
      })
      .then(function (body) {
        var lower = term.toLowerCase();
        return {
          users: body.users,
          groups: allowed.groups
            .filter(function (name) {
              return name.toLowerCase().indexOf(lower) !== -1;
            })
            .map(function (name) {
              return { name: name };
            }),
        };
      });
  }

  // The assign modal for a target: assignee, status, note.
  function openModal(targetType, targetId, editing) {
    var template = document.getElementById("assign-modal-template");
    if (!template) {
      return;
    }
    closeModal();
    modal = template.content.firstElementChild.cloneNode(true);
    document.body.appendChild(modal);
    var title = modal.querySelector(".d-modal__title-text");
    title.textContent = editing
      ? title.dataset.titleEdit
      : targetType === "Post"
        ? title.dataset.titlePost
        : title.dataset.titleTopic;
    var input = modal.querySelector("#assignee");
    var list = modal.querySelector("#assign-suggestions");
    var groups = {};
    var remember = function (body) {
      (body.groups || []).forEach(function (g) {
        groups[g.name] = true;
      });
      fill(list, body.suggestions || body.users, body.groups);
    };
    query(targetType, targetId).then(remember);
    var timer = null;
    input.addEventListener("input", function () {
      clearTimeout(timer);
      var term = input.value.trim();
      timer = setTimeout(function () {
        query(targetType, targetId, term).then(remember);
      }, 250);
    });
    modal.querySelectorAll(".modal-close").forEach(function (b) {
      b.addEventListener("click", closeModal);
    });
    modal.querySelector(".assign-submit").addEventListener("click", function () {
      var name = input.value.trim().replace(/^@/, "");
      var error = modal.querySelector(".error-label");
      if (!name) {
        error.hidden = false;
        input.closest(".control-group").classList.add("assignee-error");
        return;
      }
      var params = { target_type: targetType, target_id: targetId };
      if (groups[name]) {
        params.group_name = name;
      } else {
        params.username = name;
      }
      var note = modal.querySelector("#assign-note").value;
      if (note) {
        params.note = note;
      }
      var status = modal.querySelector("#assign-status");
      if (status) {
        params.status = status.value;
      }
      put("/assign/assign", params).then(function (r) {
        if (r.ok) {
          closeModal();
          redraw(targetType, targetId);
        } else {
          showError(r);
        }
      });
    });
    input.focus();
  }

  function toggleMenu(trigger, templateSelector) {
    var open = trigger.parentElement.querySelector(".assign-menu-content");
    if (open) {
      open.remove();
      trigger.setAttribute("aria-expanded", "false");
      return;
    }
    var template = trigger.parentElement.querySelector(templateSelector);
    if (!template) {
      return;
    }
    var content = document.createElement("div");
    content.className = "fk-d-menu assign-menu-content";
    content.appendChild(template.content.cloneNode(true));
    trigger.after(content);
    trigger.setAttribute("aria-expanded", "true");
  }

  document.addEventListener("click", function (event) {
    var target = event.target.closest(
      "#topic-footer-button-assign, .post-action-menu__assign-post, .post-action-menu__unassign-post, " +
        ".unassign-btn, .edit-assignment-btn, .reassign-item, .reassign-trigger, " +
        ".post-assign-menu-trigger, .unassign-mobile-header"
    );
    if (!target) {
      return;
    }
    event.preventDefault();
    var type = target.dataset.targetType;
    var id = target.dataset.targetId;
    if (target.classList.contains("reassign-trigger")) {
      toggleMenu(target, ".reassign-menu");
    } else if (target.classList.contains("post-assign-menu-trigger")) {
      toggleMenu(target, ".post-assign-menu");
    } else if (
      target.classList.contains("post-action-menu__unassign-post") ||
      target.classList.contains("unassign-btn") ||
      target.classList.contains("unassign-mobile-header")
    ) {
      unassign(type, id);
    } else if (target.classList.contains("reassign-item")) {
      switch (target.dataset.action) {
        case "unassign":
          unassign(type, id);
          break;
        case "reassign-self":
          assignTo(type, id, target.dataset.username);
          break;
        default:
          openModal(type, id, true);
      }
    } else {
      openModal(type, id, target.classList.contains("edit-assignment-btn"));
    }
  });

  document.addEventListener("keydown", function (event) {
    if (event.key === "Escape") {
      closeModal();
    }
  });
})();
