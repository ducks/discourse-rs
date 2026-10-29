// discourse-policy's PostPolicy component in the browser (what its clicks
// do; crate::plugins::policy::view draws it). Accept and revoke send PUT
// /policy/accept and /policy/unaccept, then the component is drawn again
// (GET /live/policy/<post id>), as the policy_change on the topic's
// stream redraws the post for everyone else. The toggles switch between
// who accepted and who has not (showNotAccepted), drawn by the server;
// load more appends the next page of avatars from /policy/accepted.json
// or /policy/not-accepted.json, as Ember pushes them onto the post. Not
// ported yet, and refused when clicked: the settings button's policy
// builder.
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.once && !Discourse.once("policy")) {
    return;
  }
  var script = document.currentScript;

  function basePath() {
    var meta = document.querySelector('meta[name="discourse-base-uri"]');
    return meta ? meta.content : "";
  }

  function headers(json) {
    var out = {};
    try {
      out = JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (e) {
      // No token: the server refuses the request and says so.
    }
    out["X-Requested-With"] = "XMLHttpRequest";
    out.Accept = "application/json";
    if (!json) {
      out["Content-Type"] = "application/x-www-form-urlencoded";
    }
    return out;
  }

  function alertError(message) {
    if (Discourse.dialog && Discourse.dialog.alert) {
      Discourse.dialog.alert(message);
    } else {
      window.alert(message);
    }
  }

  // popupAjaxError
  function failed(response) {
    return response
      .json()
      .then(function (body) {
        alertError((body.errors || [response.statusText]).join("\n"));
      })
      .catch(function () {
        alertError(response.statusText);
      });
  }

  // The list each footer shows, by post id.
  var showNotAccepted = {};

  function redraw(footer) {
    var postId = footer.dataset.postId;
    var url =
      basePath() + "/live/policy/" + postId + (showNotAccepted[postId] ? "?not_accepted=1" : "");
    return fetch(url, { credentials: "same-origin" }).then(function (response) {
      if (!response.ok) {
        return;
      }
      return response.text().then(function (html) {
        var template = document.createElement("template");
        template.innerHTML = html.trim();
        var fresh = template.content.firstElementChild;
        if (fresh && footer.isConnected) {
          footer.replaceWith(fresh);
        }
      });
    });
  }

  function updatePolicy(footer, button, action) {
    if (button.disabled) {
      return;
    }
    footer.querySelectorAll(".policy-actions .btn").forEach(function (b) {
      b.disabled = true;
      b.classList.add("is-loading");
    });
    fetch(basePath() + "/policy/" + action, {
      method: "PUT",
      credentials: "same-origin",
      headers: headers(false),
      body: new URLSearchParams({ post_id: footer.dataset.postId }).toString(),
    })
      .then(function (response) {
        return response.ok ? undefined : failed(response);
      })
      .finally(function () {
        redraw(footer);
      });
  }

  function avatar(user, size) {
    var img = document.createElement("img");
    img.setAttribute("loading", "lazy");
    img.setAttribute("alt", "");
    img.setAttribute("width", "20");
    img.setAttribute("height", "20");
    img.setAttribute("src", (user.avatar_template || "").replace("{size}", size));
    img.className = "avatar";
    return img;
  }

  function loadMore(footer, link) {
    var list = link.closest(".users");
    var accepted = list.classList.contains("accepted");
    var shown = list.querySelectorAll("img.avatar");
    var size = "24";
    if (shown.length) {
      var match = /\/(\d+)\/[^/]*$/.exec(shown[0].getAttribute("src") || "");
      if (match) {
        size = match[1];
      }
    }
    var toggle = footer.querySelector(accepted ? ".toggle-accepted" : ".toggle-not-accepted");
    var total = toggle ? parseInt(toggle.querySelector(".user-count").textContent, 10) : 0;
    var params = new URLSearchParams({ post_id: footer.dataset.postId, offset: shown.length });
    var path = accepted ? "/policy/accepted.json" : "/policy/not-accepted.json";
    fetch(basePath() + path + "?" + params.toString(), {
      credentials: "same-origin",
      headers: headers(true),
    })
      .then(function (response) {
        if (!response.ok) {
          return failed(response);
        }
        return response.json().then(function (body) {
          (body.users || []).forEach(function (user) {
            list.insertBefore(avatar(user, size), link);
          });
          var remaining = total - list.querySelectorAll("img.avatar").length;
          if (remaining > 0) {
            link.firstChild.textContent = " + " + remaining + " ";
          } else {
            link.remove();
          }
        });
      })
      .catch(function () {
        alertError("Something went wrong.");
      });
  }

  document.addEventListener("click", function (event) {
    var footer = event.target.closest(".policy-footer[data-post-id]");
    if (!footer) {
      return;
    }
    var accept = event.target.closest(".btn-accept-policy");
    if (accept) {
      updatePolicy(footer, accept, "accept");
      return;
    }
    var revoke = event.target.closest(".btn-revoke-policy");
    if (revoke) {
      updatePolicy(footer, revoke, "unaccept");
      return;
    }
    var toggle = event.target.closest(".toggle");
    if (toggle) {
      event.preventDefault();
      var postId = footer.dataset.postId;
      showNotAccepted[postId] = !showNotAccepted[postId];
      redraw(footer);
      return;
    }
    var more = event.target.closest(".load-more-users");
    if (more) {
      event.preventDefault();
      loadMore(footer, more);
      return;
    }
    if (event.target.closest(".edit-policy-settings-btn")) {
      openBuilder(footer.dataset.postId, null);
    }
  });

  // modal/policy-builder.gjs: POLICY_MARKDOWN_FIELDS, in order.
  var MARKDOWN_FIELDS = [
    ["groups", "groups"],
    ["version", "version"],
    ["renew", "renew"],
    ["renewStart", "renew-start"],
    ["reminder", "reminder"],
    ["accept", "accept"],
    ["revoke", "revoke"],
    ["addUsersToGroup", "add-users-to-group"],
  ];

  function isPresent(value) {
    return value !== null && value !== undefined && String(value).trim() !== "";
  }

  function formData(form) {
    var data = {};
    MARKDOWN_FIELDS.forEach(function (pair) {
      var el = form.elements[pair[0]];
      if (!el) {
        return;
      }
      if (el.multiple) {
        data[pair[0]] = Array.prototype.filter
          .call(el.options, function (o) {
            return o.selected;
          })
          .map(function (o) {
            return o.value;
          })
          .join(",");
      } else {
        data[pair[0]] = el.value === "__NONE__" ? null : el.value;
      }
    });
    data.private = !!(form.elements["private"] && form.elements["private"].checked);
    return data;
  }

  function markdownParams(data) {
    var params = [];
    MARKDOWN_FIELDS.forEach(function (pair) {
      if (isPresent(data[pair[0]])) {
        params.push(pair[1] + '="' + data[pair[0]] + '"');
      }
    });
    if (data.private) {
      params.push('private="true"');
    }
    return params.join(" ");
  }

  // PolicyBuilderForm#validatePolicy, its errors under their fields.
  function validate(root, form, data) {
    form.querySelectorAll(".form-kit__errors").forEach(function (e) {
      e.remove();
    });
    var errors = [];
    if (!isPresent(data.groups)) {
      errors.push(["groups", root.dataset.errorGroup]);
    }
    if (!isPresent(data.version)) {
      errors.push(["version", root.dataset.errorVersion]);
    }
    errors.forEach(function (pair) {
      var field = form.querySelector('[data-name="' + pair[0] + '"] .form-kit__container-content');
      if (field) {
        var p = document.createElement("p");
        p.className = "form-kit__errors";
        p.textContent = pair[1];
        field.appendChild(p);
      }
    });
    return errors.length === 0;
  }

  // updatePolicy: the post's first [policy ...] tag rewritten, saved with
  // the edit reason; nothing saved without one.
  function savePolicy(root, postId, data) {
    return fetch(root.dataset.rawUrl, { credentials: "same-origin", headers: headers(true) })
      .then(function (response) {
        return response.ok ? response.json() : failed(response).then(function () {});
      })
      .then(function (post) {
        if (!post || typeof post.raw !== "string") {
          return;
        }
        var regex = /\[policy\s(.*?)\]/m;
        if (!regex.test(post.raw)) {
          return;
        }
        var raw = post.raw.replace(regex, "[policy " + markdownParams(data) + "]");
        return fetch(basePath() + "/posts/" + postId, {
          method: "PUT",
          credentials: "same-origin",
          headers: headers(false),
          body: new URLSearchParams({
            "post[raw]": raw,
            "post[edit_reason]": root.dataset.editReason,
          }).toString(),
        }).then(function (response) {
          return response.ok ? undefined : failed(response);
        });
      });
  }

  // modal/policy-builder.gjs: drawn by the server (GET
  // /live/policy-builder), to insert a policy at the composer's cursor
  // (addText) or edit the post's.
  function openBuilder(postId, addText) {
    var url = basePath() + "/live/policy-builder" + (postId ? "?post_id=" + postId : "");
    fetch(url, { credentials: "same-origin" })
      .then(function (response) {
        if (!response.ok) {
          return failed(response);
        }
        return response.text().then(function (html) {
          var template = document.createElement("template");
          template.innerHTML = html.trim();
          var root = template.content.firstElementChild;
          var form = root.querySelector("form");
          var footer = root.querySelector(".policy-builder__footer");
          var button = footer.querySelector("button");
          form.addEventListener("submit", function (event) {
            event.preventDefault();
          });
          var close = Discourse.modal.show({
            className: "policy-builder",
            title: root.dataset.title,
            body: function (el) {
              el.appendChild(root);
            },
            footer: function (el) {
              el.appendChild(button);
            },
          });
          footer.remove();
          button.addEventListener("click", function () {
            var data = formData(form);
            if (!validate(root, form, data)) {
              return;
            }
            if (root.dataset.insert === "true") {
              addText(
                "\n\n" +
                  "[policy " +
                  markdownParams(data) +
                  "]\n" +
                  root.dataset.template +
                  "\n[/policy]" +
                  "\n\n"
              );
              close();
              return;
            }
            button.disabled = true;
            button.classList.add("is-loading");
            savePolicy(root, postId, data).finally(function () {
              close();
            });
          });
        });
      })
      .catch(function () {
        alertError("Something went wrong.");
      });
  }

  // attachPolicy on the composer's preview (no post): the content in
  // div.policy-body, a "Policy" header before it.
  var title = script ? script.dataset.title : "Policy";
  Discourse.policyPreview = function (preview) {
    if (!Discourse.cooked || !Discourse.cooked.setting("policy_enabled")) {
      return;
    }
    var policy = preview.querySelector(".policy");
    if (!policy) {
      return;
    }
    policy.innerHTML = '<div class="policy-body">' + policy.innerHTML + "</div>";
    var header = document.createElement("div");
    header.classList.add("policy-preview");
    header.innerText = title;
    policy.prepend(header);
  };

  // The composer's options menu item (add-policy-builder).
  Discourse.policyBuilder = {
    show: function (addText) {
      openBuilder(null, addText);
    },
  };
})();
