// AvatarSelectorModal (modal/avatar-selector.gjs), for the preferences'
// account tab (static/js/preferences.js) and, with deferSave, for a new
// account's avatar on the invite page (static/js/invite.js).
(function () {
  "use strict";

  if (!Discourse.once("avatar-selector")) {
    return;
  }

  // The deferred picker's acceptedFormatsOverride.
  var ACCEPTED = ".png,.jpg,.jpeg,.gif,.svg,.ico,.heic,.heif,.webp,.avif,.jxl";

  function icon(name) {
    return (
      '<svg class="fa d-icon d-icon-' +
      name +
      ' svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name +
      '"></use></svg>'
    );
  }

  function escape(text) {
    var div = document.createElement("div");
    div.textContent = text == null ? "" : String(text);
    return div.innerHTML.replace(/"/g, "&quot;");
  }

  function base() {
    return (document.querySelector('meta[name="discourse-base-uri"]') || {}).content || "";
  }

  function csrf() {
    return (document.querySelector('meta[name="csrf-token"]') || {}).content || "";
  }

  // jQuery.param over a flat object: undefined is no param, null an
  // empty one.
  function param(data) {
    return Object.keys(data)
      .filter(function (k) {
        return data[k] !== undefined;
      })
      .map(function (k) {
        return encodeURIComponent(k) + "=" + encodeURIComponent(data[k] === null ? "" : data[k]);
      })
      .join("&")
      .replace(/%20/g, "+");
  }

  function ajax(method, url, data) {
    return fetch(base() + url, {
      method: method,
      credentials: "same-origin",
      headers: {
        "Content-Type": "application/x-www-form-urlencoded; charset=UTF-8",
        "X-CSRF-Token": csrf(),
        "X-Requested-With": "XMLHttpRequest",
        Accept: "application/json, text/javascript, */*; q=0.01",
      },
      body: data ? param(data) : undefined,
    }).then(function (response) {
      return response.text().then(function (text) {
        var json = null;
        try {
          json = text ? JSON.parse(text) : null;
        } catch (e) {
          json = null;
        }
        if (!response.ok) {
          throw new Error((json && json.errors && json.errors.join("\n")) || (json && json.error) || "");
        }
        return json;
      });
    });
  }

  // avatarUrl over getRawSize: the template at the size times the pixel
  // ratio (1, 2 or 3), drawn as dBoundAvatarTemplate draws it.
  function boundAvatar(template, size) {
    if (!template) {
      return '<div class="avatar-placeholder"></div>';
    }
    var ratio = window.devicePixelRatio || 1;
    var raw = ratio > 1.1 && ratio < 2.1 ? 2 : ratio >= 2.1 ? 3 : 1;
    var src = template.replace(/\{size\}/g, String(size * raw));
    if (src.charAt(0) === "/" && src.charAt(1) !== "/") {
      src = base() + src;
    }
    return '<img alt="" width="' + size + '" height="' + size + '" src="' + escape(src) + '" class="avatar">';
  }

  // lib/uploads.js, for a visitor (no staff extensions).
  function isImage(path) {
    return /\.(png|webp|jpe?g|gif|svg|ico|heic|heif|avif|jxl)$/i.test(path);
  }

  function imagesExtensions(authorized) {
    return authorized
      .toLowerCase()
      .replace(/[\s.]+/g, "")
      .split("|")
      .filter(function (ext) {
        return ext.indexOf("*") === -1 && isImage("." + ext);
      });
  }

  // validateUploadedFile(file, { imagesOnly: true }) with no user.
  function validImage(file, authorized, l) {
    var all = authorized.indexOf("*") !== -1;
    if (
      !all &&
      !authorized.split("|").filter(function (ext) {
        return ext;
      }).length
    ) {
      Discourse.dialog.alert(escape(l.no_uploads_authorized));
      return false;
    }
    var name = file && file.name;
    if (!name) {
      return false;
    }
    var images = imagesExtensions(authorized);
    if (!isImage(name) && !new RegExp("\\.(" + images.join("|") + ")$", "i").test(name)) {
      Discourse.dialog.alert(
        escape(
          l.upload_not_authorized.replace(
            "%{authorized_extensions}",
            all ? "png, jpg, jpeg, gif, svg, ico, heic, heif, webp, avif, jxl" : images.join(", ")
          )
        )
      );
      return false;
    }
    if (file.size === 0) {
      Discourse.dialog.alert(escape(l.file_size_zero));
      return false;
    }
    return true;
  }

  // modal/avatar-selector.gjs: the selectable avatars, the system,
  // gravatar and uploaded choices with AvatarUploader, Save Changes
  // through PUT .../preferences/avatar/pick and a selectable avatar
  // through .../avatar/select, each followed by a reload. With deferSave
  // (CodeLoginForm's new account) nothing is saved: the file is picked
  // with DPickFilesButton and Save hands onSelect the selection, null
  // for the system avatar, { url } for a selectable one, { file } for
  // an upload.
  function avatarSelector(data) {
    var defer = !!data.deferSave;
    var user = data.user || {};
    var l = data.labels;
    var userPath = defer ? "" : "/u/" + encodeURIComponent(String(data.username).toLowerCase());
    var pendingFile = defer && data.selection ? data.selection.file : null;
    var filePreview = pendingFile ? URL.createObjectURL(pendingFile) : null;
    var systemTemplate = defer ? data.avatarTemplate : user.system_avatar_template;
    var allowUpload = defer ? !!data.canUploadAvatar : data.allow_upload;
    var trustLevel = defer ? data.trustLevel : user.trust_level;
    function clearFilePreview() {
      if (filePreview) {
        URL.revokeObjectURL(filePreview);
        filePreview = null;
      }
    }
    var mode = data.selectable_avatars_mode;
    var showSelectable = mode !== "disabled";
    var showCustom = (function () {
      switch (mode) {
        case "no_one":
          return false;
        case "tl1":
        case "tl2":
        case "tl3":
        case "tl4":
          return !!(user.admin || user.moderator || (trustLevel || 0) >= parseInt(mode.replace("tl", ""), 10));
        case "staff":
          return !!(user.admin || user.moderator);
        default:
          return true;
      }
    })();
    var selected;
    if (defer) {
      selected = (data.selection && data.selection.url) || (pendingFile ? "custom" : "system");
    } else if (user.use_logo_small_as_avatar) {
      selected = "logo";
    } else if (user.avatar_template === user.system_avatar_template) {
      selected = "system";
    } else if (user.avatar_template === user.gravatar_avatar_template) {
      selected = "gravatar";
    } else {
      selected = "custom";
    }
    var bodyEl;
    var saveButton;

    function radio(id, name, value) {
      return (
        '<input name="' + name + '" id="' + id + '" class="ember-view" type="radio" value="' + value + '"' +
        (selected === value ? " checked" : "") +
        ">"
      );
    }

    function select(value) {
      selected = value;
      bodyEl.querySelectorAll('input[type="radio"]').forEach(function (input) {
        input.checked = input.value === value;
      });
      bodyEl.querySelectorAll(".selectable-avatar").forEach(function (a) {
        if (a.dataset.url === value) {
          a.setAttribute("aria-current", "true");
        } else {
          a.removeAttribute("aria-current");
        }
      });
      if (saveButton) {
        saveButton.disabled = submitDisabled();
      }
    }

    function submitDisabled() {
      return selected === "logo" || (defer && selected === "custom" && !pendingFile);
    }

    function uploadedChoice() {
      if (defer) {
        return (
          boundAvatar(filePreview, 48) + " " + escape(filePreview ? l.uploaded_avatar : l.uploaded_avatar_empty)
        );
      }
      return user.custom_avatar_template
        ? boundAvatar(user.custom_avatar_template, 48) + " " + escape(l.uploaded_avatar)
        : escape(l.uploaded_avatar_empty);
    }

    function render() {
      var html = "";
      if (showSelectable) {
        html += '<div class="selectable-avatars">';
        (data.selectable_avatars || []).forEach(function (url) {
          html +=
            '<a' +
            (selected === url ? ' aria-current="true"' : "") +
            ' class="selectable-avatar" href data-url="' +
            escape(url) +
            '">' +
            boundAvatar(url, 144) +
            "</a>";
        });
        html += "</div>";
        if (showCustom) {
          html += "<h4>" + escape(l.use_custom) + "</h4>";
        }
      }
      if (showCustom) {
        if (user.use_logo_small_as_avatar) {
          html +=
            '<div class="avatar-choice">' +
            radio("logo-small", "logo", "logo") +
            '<label class="radio" for="logo-small">' +
            boundAvatar(data.site_logo_small_url, 48) +
            " " +
            escape(l.logo_small) +
            "</label></div>";
        }
        html +=
          '<div class="avatar-choice avatar-choice--system">' +
          radio("system-avatar", "avatar", "system") +
          '<label class="radio" for="system-avatar">' +
          (systemTemplate
            ? boundAvatar(systemTemplate, 48)
            : '<span class="avatar-selector__placeholder">' + icon("user") + "</span>") +
          " " +
          escape(l.letter_based) +
          "</label></div>";
        if (!defer && data.allow_gravatar) {
          html +=
            '<div class="avatar-choice avatar-choice--gravatar">' +
            radio("gravatar", "avatar", "gravatar") +
            '<label class="radio" for="gravatar">' +
            boundAvatar(user.gravatar_avatar_template, 48) +
            "<span>" +
            l.gravatar +
            " " +
            escape(user.email || "") +
            "</span></label>" +
            '<button class="btn no-text btn-icon btn-default avatar-selector-refresh-gravatar" title="' +
            escape(l.refresh_gravatar_title) +
            '" type="button">' +
            icon("arrows-rotate") +
            '<span aria-hidden="true">&#8203;</span></button>' +
            '<p class="error hidden">' +
            escape(l.gravatar_failed) +
            "</p></div>";
        }
        if (allowUpload && defer) {
          // DPickFilesButton with the selector's formats, or any where
          // every extension is authorized.
          html +=
            '<div class="avatar-choice avatar-choice--upload">' +
            radio("uploaded-avatar", "avatar", "custom") +
            '<label class="radio" for="uploaded-avatar">' +
            uploadedChoice() +
            "</label>" +
            '<div class="ember-view pick-files-button"><button class="btn btn-icon-text btn-default" type="button">' +
            icon("upload") +
            '<span class="d-button-label">' +
            escape(l.upload) +
            "</span></button><input" +
            (data.authorized_extensions.indexOf("*") !== -1 ? "" : ' accept="' + ACCEPTED + '"') +
            ' class="hidden-upload-field" id="deferred-avatar-upload" type="file"></div></div>';
        } else if (allowUpload) {
          html +=
            '<div class="avatar-choice avatar-choice--upload">' +
            radio("uploaded-avatar", "avatar", "custom") +
            '<label class="radio" for="uploaded-avatar">' +
            uploadedChoice() +
            "</label>" +
            '<span class="avatar-uploader"><input accept="image/*" aria-hidden="true" class="hidden-upload-field" id="custom-profile-upload" type="file">' +
            '<button class="btn btn-icon-text btn-default avatar-uploader__button" title="' +
            escape(l.upload_title) +
            '" type="button">' +
            icon("far-image") +
            '<span class="d-button-label">' +
            escape(l.upload) +
            "</span></button></span></div>";
        }
      }
      bodyEl.innerHTML = html;
    }

    function reloadAfter(promise) {
      promise
        .then(function () {
          window.location.reload();
        })
        .catch(function (e) {
          Discourse.dialog.popupAjaxError(e.message || l.generic_error || "Sorry, an error has occurred.");
        });
    }

    Discourse.modal.show({
      onClose: clearFilePreview,
      className: "avatar-selector-modal",
      title: l.title,
      body: function (el) {
        bodyEl = el;
        el.classList.add("avatar-selector");
        render();
        el.addEventListener("click", function (event) {
          var choice = event.target.closest('input[type="radio"]');
          if (choice) {
            select(choice.value);
            return;
          }
          var selectable = event.target.closest(".selectable-avatar");
          if (selectable) {
            event.preventDefault();
            if (defer) {
              select(selectable.dataset.url);
              return;
            }
            reloadAfter(ajax("PUT", userPath + "/preferences/avatar/select", { url: selectable.dataset.url }));
            return;
          }
          if (event.target.closest(".pick-files-button .btn")) {
            el.querySelector("#deferred-avatar-upload").click();
            return;
          }
          if (event.target.closest(".avatar-uploader__button")) {
            el.querySelector("#custom-profile-upload").click();
            return;
          }
          var refresh = event.target.closest(".avatar-selector-refresh-gravatar");
          if (refresh) {
            refresh.disabled = true;
            ajax("POST", "/user_avatar/" + encodeURIComponent(data.username) + "/refresh_gravatar.json")
              .then(function (result) {
                var failed = el.querySelector(".avatar-choice--gravatar .error");
                if (!result || !result.gravatar_upload_id) {
                  failed.classList.remove("hidden");
                } else {
                  failed.classList.add("hidden");
                  user.gravatar_avatar_upload_id = result.gravatar_upload_id;
                  user.gravatar_avatar_template = result.gravatar_avatar_template;
                  var label = el.querySelector('label[for="gravatar"]');
                  var old = label.querySelector(".avatar, .avatar-placeholder");
                  old.outerHTML = boundAvatar(user.gravatar_avatar_template, 48);
                }
              })
              .finally(function () {
                refresh.disabled = false;
              });
          }
        });
        el.addEventListener("change", function (event) {
          var picked = event.target.closest("#deferred-avatar-upload");
          if (picked) {
            // DPickFilesButton#_filesPicked, then filesPicked.
            var file0 = picked.files[0];
            if (!file0) {
              return;
            }
            if (
              data.authorized_extensions.indexOf("*") === -1 &&
              ACCEPTED.indexOf("." + file0.name.split(".").pop()) === -1 &&
              ACCEPTED.indexOf(file0.type) === -1
            ) {
              // acceptedFileTypesString is not passed: no types named.
              Discourse.dialog.alert(escape(l.unsupported_file_picked.replace("%{types}", "")));
              return;
            }
            if (!validImage(file0, data.authorized_extensions, l)) {
              return;
            }
            clearFilePreview();
            pendingFile = file0;
            filePreview = URL.createObjectURL(file0);
            el.querySelector('label[for="uploaded-avatar"]').innerHTML = uploadedChoice();
            select("custom");
            return;
          }
          var input = event.target.closest("#custom-profile-upload");
          if (!input || !input.files.length) {
            return;
          }
          var file = input.files[0];
          var button = el.querySelector(".avatar-uploader__button");
          var label = button.querySelector(".d-button-label");
          var form = new FormData();
          form.append("upload_type", "avatar");
          form.append("user_id", user.id || "");
          form.append("file", file, file.name);
          var xhr = new XMLHttpRequest();
          xhr.open("POST", base() + "/uploads.json?client_id=" + Date.now());
          xhr.setRequestHeader("X-CSRF-Token", csrf());
          xhr.setRequestHeader("X-Requested-With", "XMLHttpRequest");
          input.disabled = true;
          button.disabled = true;
          xhr.upload.addEventListener("progress", function (e) {
            if (e.lengthComputable) {
              label.textContent = l.uploading + " " + Math.round((e.loaded / e.total) * 100) + "%";
            }
          });
          xhr.addEventListener("loadend", function () {
            input.disabled = false;
            button.disabled = false;
            label.textContent = l.upload;
            input.value = "";
            var json = null;
            try {
              json = JSON.parse(xhr.responseText);
            } catch (e) {
              json = null;
            }
            if (xhr.status >= 200 && xhr.status < 300 && json && json.id) {
              user.custom_avatar_template = json.url;
              user.custom_avatar_upload_id = json.id;
              el.querySelector('label[for="uploaded-avatar"]').innerHTML = uploadedChoice();
              button.dataset.avatarUploadId = json.id;
              button.dataset.uploaded = "true";
              var warning = el.querySelector(".avatar-uploader .warning");
              if (json.width !== json.height) {
                if (!warning) {
                  button.insertAdjacentHTML(
                    "afterend",
                    '<div class="warning">' + escape(l.image_is_not_a_square) + "</div>"
                  );
                }
              } else if (warning) {
                warning.remove();
              }
              select("custom");
            } else {
              Discourse.dialog.popupAjaxError(
                (json && json.errors && json.errors.join(" ")) || l.generic_error || "Sorry, an error has occurred."
              );
            }
          });
          xhr.send(form);
        });
      },
      footer: function (el, close) {
        if (!showCustom && !defer) {
          return;
        }
        el.innerHTML =
          '<button class="btn btn-primary" type="button"><span class="d-button-label">' +
          escape(l.save) +
          '</span></button><button class="btn btn-flat d-modal-cancel" type="button"><span class="d-button-label">' +
          escape(l.cancel) +
          "</span></button>";
        saveButton = el.querySelector(".btn-primary");
        saveButton.disabled = submitDisabled();
        saveButton.addEventListener("click", function () {
          if (defer) {
            var selection = null;
            if (selected === "custom") {
              selection = { file: pendingFile };
            } else if (selected !== "system") {
              selection = { url: selected };
            }
            data.onSelect(selection);
            close();
            return;
          }
          var uploadId =
            selected === "system"
              ? user.system_avatar_upload_id
              : selected === "gravatar"
                ? user.gravatar_avatar_upload_id
                : user.custom_avatar_upload_id;
          reloadAfter(ajax("PUT", userPath + "/preferences/avatar/pick", { upload_id: uploadId, type: selected }));
        });
        el.querySelector(".d-modal-cancel").addEventListener("click", close);
      },
    });
  }

  Discourse.avatarSelector = avatarSelector;
  Discourse.boundAvatar = boundAvatar;
})();
