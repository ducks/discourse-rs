// The logo and fonts pages in the browser (src/admin_branding_view.rs
// draws them). AdminLogoForm: each image field's upload (POST /uploads,
// type branding) and delete, the lightbox, the dark logos' toggles, the
// collapsed sections, the manifest screenshots' SimpleList, the save
// (PUT /admin/config/logo) and the logo it puts in the header.
// AdminFontsForm: the font and text size choices, More fonts, the
// default text size's user count and backfill question, the save (PUT
// /admin/config/fonts) and the fonts and size it applies to the page.
(function () {
  "use strict";

  if (!Discourse.once("admin-branding")) {
    return;
  }

  function ui() {
    return Discourse.adminUi;
  }

  function t(key, args) {
    return ui().t(key, args);
  }

  function esc(s) {
    return ui().escapeHtml(s);
  }

  // --- AdminLogoForm ---------------------------------------------------------

  function logoForm() {
    // The forms with image fields: the logo's and the welcome banner's.
    return document.querySelector("form.admin-logo-form, form.admin-welcome-banner-form");
  }

  // formData: the values given, changed as the fields are.
  function values(form) {
    try {
      return JSON.parse(form.dataset.values || "{}");
    } catch (_) {
      return {};
    }
  }

  function setValue(form, name, value) {
    var v = values(form);
    v[name] = value;
    form.dataset.values = JSON.stringify(v);
  }

  // UppyImageUploader, drawn again for the field's value.
  function drawUploader(uploader, url) {
    var id = uploader.id;
    var input = id + "__input";
    var placeholder = uploader.dataset.placeholder;
    var picker = function (cls, label) {
      return (
        '<label class="' +
        cls +
        '" for="' +
        esc(input) +
        '" tabindex="0">' +
        ui().icon("upload") +
        '<div class="pick-files-button"><input accept="image/*" id="' +
        esc(input) +
        '" type="file"></div>' +
        esc(t(label)) +
        "</label>"
      );
    };
    var html;
    if (!url) {
      var uploading = esc(t("upload_selector.uploading"));
      html =
        '<div class="file-uploader__preview input-xxlarge" style="background-image: url()">' +
        (placeholder
          ? '<div class="placeholder-overlay" style="background-image: url(' + esc(placeholder) + ')"></div>'
          : "") +
        '<div class="file-uploader__controls">' +
        picker("btn btn-transparent", "upload_selector.select_file") +
        '<div class="file-uploader__progress-status hidden"><div aria-label="' +
        uploading +
        ' 0%" class="progress-bar-container" role="progressbar"><div class="progress-bar" style="width: 0%"></div></div><span>' +
        uploading +
        " 0%</span></div></div></div>";
    } else {
      html =
        '<div class="file-uploader__preview input-xxlarge" style="background-image: url(' +
        esc(url) +
        ')"><a class="lightbox" href="' +
        esc(url) +
        '" rel="nofollow ugc noopener"><div class="meta"><span class="informations">x</span></div></a><div class="expand-overlay"><button class="btn no-text btn-icon btn-default btn-small image-uploader-lightbox-btn" title="' +
        esc(t("expand")) +
        '" type="button">' +
        ui().icon("discourse-expand") +
        '<span aria-hidden="true">&#8203;</span></button></div></div><div class="file-uploader__controls">' +
        picker("btn btn-default btn-small", "upload_selector.change") +
        '<button class="btn btn-icon-text btn-danger btn-small" type="button">' +
        ui().icon("trash-can") +
        '<span class="d-button-label">' +
        esc(t("upload_selector.delete")) +
        "</span></button></div>";
    }
    uploader.innerHTML = html;
    uploader.classList.toggle("has-image", !!url);
    uploader.classList.toggle("no-image", !url);
    lightbox(uploader);
  }

  function lightbox(uploader) {
    if (Discourse.lightbox && uploader.classList.contains("has-image")) {
      Discourse.lightbox(uploader);
    }
  }

  function upload(form, container, file) {
    var name = container.dataset.name;
    var uploader = container.querySelector(".file-uploader");
    var status = uploader.querySelector(".file-uploader__progress-status");
    if (status) {
      status.classList.remove("hidden");
    }
    var body = new FormData();
    body.append("upload_type", form.dataset.uploadType || "branding");
    if (name === "favicon") {
      body.append("for_site_setting", "true");
      body.append("site_setting_name", "favicon");
    }
    body.append("file", file);
    ui()
      .ajax("POST", "/uploads.json", body)
      .then(function (result) {
        var url = ui().base() + result.url;
        setValue(form, name, url);
        drawUploader(uploader, url);
      })
      .catch(function (error) {
        if (status) {
          status.classList.add("hidden");
        }
        ui().failed(error);
      });
  }

  function setToggle(button, on) {
    button.setAttribute("aria-checked", on ? "true" : "false");
    button.parentNode.querySelector(".d-toggle-switch__checkbox-slider").innerHTML = on ? ui().icon("check") : "";
  }

  // A dark logo's field shows while its toggle is on.
  function toggleDark(form, container) {
    var button = container.querySelector(".d-toggle-switch__checkbox");
    var on = button.getAttribute("aria-checked") !== "true";
    setToggle(button, on);
    var name = container.dataset.name;
    var template = form.querySelector("template.admin-logo-form__dark[data-for='" + name + "']");
    if (on && template) {
      container.insertAdjacentElement("afterend", template.content.firstElementChild.cloneNode(true));
      template.remove();
      var section = container.nextElementSibling;
      section.querySelectorAll(".file-uploader").forEach(lightbox);
    } else if (!on) {
      var shown = container.nextElementSibling;
      if (shown && shown.classList.contains("form-kit__section")) {
        var t2 = document.createElement("template");
        t2.className = "admin-logo-form__dark";
        t2.dataset.for = name;
        t2.content.appendChild(shown.cloneNode(true));
        shown.replaceWith(t2);
      }
    }
  }

  // AdminConfigAreaCardSection: open or close a collapsable section.
  function toggleSection(header) {
    var section = header.closest(".admin-config-area-card-section");
    var content = section.querySelector(":scope > .admin-config-area-card-section__content");
    var template = section.querySelector(":scope > template.admin-config-area-card-section__template");
    var open;
    if (!content && template) {
      section.appendChild(template.content.firstElementChild.cloneNode(true));
      template.remove();
      content = section.querySelector(":scope > .admin-config-area-card-section__content");
      content.querySelectorAll(".file-uploader").forEach(lightbox);
      open = true;
    } else {
      open = content.hidden;
      content.hidden = !open;
    }
    var old = header.querySelector("svg");
    old.outerHTML = ui().icon(open ? "angle-down" : "angle-right");
  }

  // The manifest screenshots' SimpleList.
  function listValues(list) {
    return Array.prototype.map.call(list.querySelectorAll(".values .value-input"), function (input) {
      return input.value;
    });
  }

  function drawList(form, list, items) {
    var old = list.querySelector(".values");
    if (old) {
      old.remove();
    }
    if (items.length) {
      var html = '<ul class="values">';
      items.forEach(function (v, i) {
        html +=
          '<li class="value" data-index="' +
          i +
          '"><button class="btn no-text btn-icon btn-default remove-value-btn btn-small" type="button">' +
          ui().icon("xmark") +
          '<span aria-hidden="true">&#8203;</span></button><input class="value-input" type="text" value="' +
          esc(v) +
          '"></li>';
      });
      list.insertAdjacentHTML("afterbegin", html + "</ul>");
    }
    setValue(form, "manifest_screenshots", items.join("|"));
  }

  document.addEventListener("change", function (event) {
    var form = logoForm();
    if (!form || !form.contains(event.target)) {
      return;
    }
    var input = event.target.closest(".pick-files-button input[type=file]");
    if (input && input.files.length) {
      upload(form, input.closest(".form-kit__field"), input.files[0]);
      input.value = "";
      return;
    }
    var value = event.target.closest(".simple-list .values .value-input");
    if (value) {
      var list = value.closest(".simple-list");
      setValue(form, "manifest_screenshots", listValues(list).join("|"));
    }
  });

  document.addEventListener("input", function (event) {
    var add = event.target.closest && event.target.closest("form.admin-logo-form .add-value-input");
    if (add) {
      add.parentNode.querySelector(".add-value-btn").disabled = add.value.trim() === "";
    }
  });

  function addListValue(form, input) {
    var list = input.closest(".simple-list");
    var value = input.value.trim();
    if (!value) {
      return;
    }
    var items = listValues(list);
    // SimpleList's inputDelimiter: several values at once.
    value.split("|").forEach(function (v) {
      if (v && items.indexOf(v) === -1) {
        items.push(v);
      }
    });
    input.value = "";
    input.parentNode.querySelector(".add-value-btn").disabled = true;
    drawList(form, list, items);
  }

  document.addEventListener("keydown", function (event) {
    var form = logoForm();
    var input = event.target.closest && event.target.closest("form.admin-logo-form .add-value-input");
    if (form && input && event.key === "Enter") {
      event.preventDefault();
      addListValue(form, input);
    }
  });

  document.addEventListener("click", function (event) {
    var form = logoForm();
    if (!form || !form.contains(event.target)) {
      return;
    }
    var target = event.target;
    var header = target.closest(".admin-config-area-card-section__header-wrapper.collapsable");
    if (header) {
      toggleSection(header);
      return;
    }
    var toggle = target.closest(".form-kit__field-toggle .d-toggle-switch__checkbox");
    if (toggle) {
      toggleDark(form, toggle.closest(".form-kit__field"));
      return;
    }
    var expand = target.closest(".image-uploader-lightbox-btn");
    if (expand) {
      var link = expand.closest(".file-uploader").querySelector("a.lightbox");
      if (link) {
        link.click();
      }
      return;
    }
    var remove = target.closest(".file-uploader__controls .btn-danger");
    if (remove) {
      var container = remove.closest(".form-kit__field");
      setValue(form, container.dataset.name, null);
      drawUploader(container.querySelector(".file-uploader"), null);
      return;
    }
    var add = target.closest(".simple-list .add-value-btn");
    if (add) {
      addListValue(form, add.parentNode.querySelector(".add-value-input"));
      return;
    }
    var removeValue = target.closest(".simple-list .remove-value-btn");
    if (removeValue) {
      var list = removeValue.closest(".simple-list");
      var items = listValues(list);
      items.splice(Number(removeValue.closest(".value").dataset.index), 1);
      drawList(form, list, items);
    }
  });

  var LOGO_FIELDS = [
    "logo",
    "logo_dark",
    "large_icon",
    "favicon",
    "logo_small",
    "logo_small_dark",
    "mobile_logo",
    "mobile_logo_dark",
    "manifest_icon",
    "manifest_screenshots",
    "apple_touch_icon",
    "digest_logo",
    "opengraph_image",
    "x_summary_large_image",
  ];

  function saveLogo(form) {
    var v = values(form);
    var data = {};
    LOGO_FIELDS.forEach(function (name) {
      data[name] = v[name] === undefined ? null : v[name];
    });
    var submit = form.querySelector("button[type=submit]");
    submit.disabled = true;
    ui()
      .ajax("PUT", "/admin/config/logo.json", data)
      .then(function () {
        ui().toast(t("admin.config.logo.form.saved"));
        // siteSettingChangeTracker.refreshPage: the header's logo.
        var logo = document.getElementById("site-logo");
        if ((!data.logo && logo) || (data.logo && !logo)) {
          location.reload();
        } else if (logo && data.logo) {
          logo.setAttribute("src", data.logo);
        }
      }, ui().failed)
      .then(function () {
        submit.disabled = false;
      });
  }

  // --- AdminFontsForm ----------------------------------------------------------

  function fontsForm() {
    return document.querySelector("form.admin-fonts-form");
  }

  function chosen(form, name) {
    var active = form.querySelector("[data-name='" + name + "'] .admin-fonts-form__button-option.active");
    return active ? active.dataset.value : "";
  }

  function fontName(button) {
    return button ? button.textContent : "";
  }

  function saveFonts(form, backfill) {
    var data = {
      base_font: chosen(form, "base_font"),
      heading_font: chosen(form, "heading_font"),
      default_text_size: chosen(form, "default_text_size"),
      update_existing_users: backfill,
    };
    ui()
      .ajax("PUT", "/admin/config/fonts.json", data)
      .then(function () {
        ui().toast(t("admin.config.fonts.form.saved"));
        form.dataset.defaultTextSize = data.default_text_size;
        // siteSettingChangeTracker.refreshPage: the fonts and size now.
        var root = document.documentElement;
        root.style.setProperty(
          "--font-family",
          fontName(form.querySelector("[data-name='base_font'] .admin-fonts-form__button-option.active"))
        );
        root.style.setProperty(
          "--heading-font-family",
          fontName(form.querySelector("[data-name='heading_font'] .admin-fonts-form__button-option.active"))
        );
        ["smallest", "smaller", "normal", "larger", "largest"].forEach(function (size) {
          root.classList.remove("text-size-" + size);
        });
        root.classList.add("text-size-" + data.default_text_size);
      }, ui().failed);
  }

  // UpdateDefaultTextSize: apply the new size to the users holding the old.
  function askBackfill(count, done) {
    var answer = null;
    var close = Discourse.modal.show({
      title: t("admin.config.fonts.backfill_modal.title"),
      body: function (el) {
        el.textContent = t("admin.config.fonts.backfill_modal.description." + (count === 1 ? "one" : "other"), {
          count: count,
        });
      },
      footer: function (el, closeModal) {
        el.innerHTML =
          '<button class="btn btn-text btn-primary" type="button"><span class="d-button-label">' +
          esc(t("admin.config.fonts.backfill_modal.modal_yes")) +
          '</span></button><button class="btn btn-text" type="button"><span class="d-button-label">' +
          esc(t("admin.config.fonts.backfill_modal.modal_no")) +
          "</span></button>";
        var buttons = el.querySelectorAll("button");
        buttons[0].addEventListener("click", function () {
          answer = true;
          closeModal();
        });
        buttons[1].addEventListener("click", function () {
          answer = null;
          closeModal();
        });
      },
      onClose: function () {
        done(answer);
      },
    });
    return close;
  }

  function submitFonts(form) {
    var size = chosen(form, "default_text_size");
    if (size === form.dataset.defaultTextSize) {
      saveFonts(form, null);
      return;
    }
    ui()
      .ajax("PUT", "/admin/site_settings/default_text_size/user_count.json", { default_text_size: size })
      .then(function (result) {
        if (result.user_count > 0) {
          askBackfill(result.user_count, function (backfill) {
            saveFonts(form, backfill);
          });
        } else {
          saveFonts(form, null);
        }
      }, ui().failed);
  }

  document.addEventListener("click", function (event) {
    var form = fontsForm();
    if (!form || !form.contains(event.target)) {
      return;
    }
    var option = event.target.closest(".admin-fonts-form__button-option");
    if (option) {
      option.parentNode.querySelectorAll(".admin-fonts-form__button-option").forEach(function (b) {
        b.classList.toggle("active", b === option);
      });
      return;
    }
    var more = event.target.closest(".admin-fonts-form__more");
    if (more) {
      var control = more.parentNode;
      var template = control.querySelector("template.admin-fonts-form__more-fonts");
      var extra = template.content.querySelectorAll("button");
      var shown = control.querySelector(".admin-fonts-form__button-option[data-value='" + extra[0].dataset.value + "']");
      if (shown) {
        Array.prototype.forEach.call(extra, function (b) {
          var el = control.querySelector(".admin-fonts-form__button-option[data-value='" + b.dataset.value + "']");
          if (el) {
            el.remove();
          }
        });
        more.textContent = more.dataset.more;
      } else {
        var selected = chosen(form, more.closest(".form-kit__field").dataset.name);
        Array.prototype.forEach.call(extra, function (b) {
          var copy = b.cloneNode(true);
          copy.classList.toggle("active", copy.dataset.value === selected);
          control.insertBefore(copy, template);
        });
        more.textContent = more.dataset.fewer;
      }
    }
  });

  // --- AdminWelcomeBannerForm ---------------------------------------------------

  function bannerForm() {
    return document.querySelector("form.admin-welcome-banner-form");
  }

  function parsed(form, key) {
    try {
      return JSON.parse(form.dataset[key] || "null");
    } catch (_) {
      return null;
    }
  }

  // The enabled themes' DMultiSelect: the loaded themes by name.
  ui().multiSelect("admin-welcome-banner-form__themes", function (term) {
    var form = bannerForm();
    var themes = (form && parsed(form, "themes")) || [];
    var filter = (term || "").toLowerCase();
    return themes.filter(function (theme) {
      return !filter || theme.name.toLowerCase().indexOf(filter) !== -1;
    });
  });

  // FKControlColor: the hex text and the picker follow each other.
  function luminanceClass(hex) {
    if (!/^([0-9a-f]{3}|[0-9a-f]{6})$/i.test(hex)) {
      return "--is-light";
    }
    var full = hex.length === 3 ? hex.replace(/(.)/g, "$1$1") : hex;
    var r = parseInt(full.slice(0, 2), 16);
    var g = parseInt(full.slice(2, 4), 16);
    var b = parseInt(full.slice(4, 6), 16);
    return (0.299 * r + 0.587 * g + 0.114 * b) / 255 > 0.5 ? "--is-light" : "--is-dark";
  }

  function syncColor(control, hex) {
    var wrapper = control.querySelector(".form-kit__control-color-picker-wrapper");
    wrapper.classList.remove("--is-light", "--is-dark");
    wrapper.classList.add(luminanceClass(hex));
  }

  document.addEventListener("input", function (event) {
    var control = event.target.closest && event.target.closest("form.admin-welcome-banner-form .form-kit__control-color");
    if (!control) {
      return;
    }
    var text = control.querySelector(".form-kit__control-color-input-hex");
    var picker = control.querySelector(".form-kit__control-color-input-picker");
    if (event.target === picker) {
      text.value = picker.value.replace(/^#/, "");
    } else if (/^([0-9a-f]{3}|[0-9a-f]{6})$/i.test(text.value.replace(/^#/, ""))) {
      var hex = text.value.replace(/^#/, "");
      picker.value = "#" + (hex.length === 3 ? hex.replace(/(.)/g, "$1$1") : hex);
    }
    syncColor(control, text.value.replace(/^#/, ""));
  });

  var TEXTS = [
    ["headerNewMembers", "js.welcome_banner.header.new_members"],
    ["headerLoggedInMembers", "js.welcome_banner.header.logged_in_members"],
    ["headerAnonymousMembers", "js.welcome_banner.header.anonymous_members"],
    ["subheaderLoggedInMembers", "js.welcome_banner.subheader.logged_in_members"],
    ["subheaderAnonymousMembers", "js.welcome_banner.subheader.anonymous_members"],
    ["searchPlaceholder", "js.welcome_banner.search_placeholder"],
  ];

  function bannerLocale(form) {
    var summary = form.querySelector(".translation-selector summary");
    return summary ? summary.dataset.value : "en";
  }

  // updateLocale: the texts in the language chosen.
  document.addEventListener("change", function (event) {
    var form = bannerForm();
    if (!form || !event.target.matches || !event.target.matches("details.translation-selector")) {
      return;
    }
    var locale = event.target.dataset.value;
    var summary = event.target.querySelector("summary");
    summary.dataset.value = locale;
    Promise.all(
      TEXTS.map(function (pair) {
        return ui()
          .ajax("GET", "/admin/customize/site_texts/" + pair[1] + ".json?locale=" + encodeURIComponent(locale))
          .then(
            function (json) {
              return json.site_text.value;
            },
            function () {
              return "";
            }
          );
      })
    ).then(function (values) {
      var original = parsed(form, "original") || {};
      TEXTS.forEach(function (pair, i) {
        form.querySelector("[name='" + pair[0] + "']").value = values[i] || "";
        original[pair[0]] = values[i] || "";
      });
      form.dataset.original = JSON.stringify(original);
    });
  });

  function bannerValue(form, name) {
    var el = form.querySelector("[name='" + name + "']");
    return el ? el.value : "";
  }

  function saveBanner(form) {
    var original = parsed(form, "original") || {};
    var current = values(form);
    var locale = bannerLocale(form);
    var chain = Promise.resolve();
    var put = function (setting, value) {
      chain = chain.then(function () {
        var data = {};
        data[setting] = value;
        return ui().ajax("PUT", "/admin/site_settings/" + setting, data);
      });
    };
    var image = current.welcomeBannerImage || "";
    if (image !== (original.welcomeBannerImage || "")) {
      put("welcome_banner_image", image);
    }
    var color = bannerValue(form, "welcomeBannerTextColor");
    if (color && /^([0-9a-f]{3}|[0-9a-f]{6})$/i.test(color)) {
      color = "#" + color;
    }
    if (color !== (original.welcomeBannerTextColor || "")) {
      put("welcome_banner_text_color", color || "");
    }
    var location = bannerValue(form, "welcomeBannerLocation");
    if (location !== original.welcomeBannerLocation) {
      put("welcome_banner_location", location);
    }
    var visibility = bannerValue(form, "welcomeBannerPageVisibility");
    if (visibility !== original.welcomeBannerPageVisibility) {
      put("welcome_banner_page_visibility", visibility);
    }
    TEXTS.forEach(function (pair) {
      var value = bannerValue(form, pair[0]);
      if (value === (original[pair[0]] || "")) {
        return;
      }
      chain = chain.then(function () {
        var path = "/admin/customize/site_texts/" + pair[1] + "?locale=" + encodeURIComponent(locale);
        return value.trim() === ""
          ? ui().ajax("DELETE", path)
          : ui().ajax("PUT", path, { site_text: { value: value, locale: locale } });
      });
    });
    chain
      .then(function () {
        var trigger = form.querySelector(".admin-welcome-banner-form__themes");
        var enabled = Array.prototype.map.call(
          trigger.querySelectorAll(".d-multi-select-trigger__selected-item"),
          function (b) {
            return b.dataset.value;
          }
        );
        // Promise.allSettled over every theme.
        return Promise.all(
          (parsed(form, "themes") || []).map(function (theme) {
            return ui()
              .ajax("PUT", "/admin/themes/" + theme.value + "/site-setting", {
                name: "enable_welcome_banner",
                value: enabled.indexOf(theme.value) !== -1,
              })
              .catch(function () {});
          })
        );
      })
      .then(function () {
        var now = Object.assign({}, original, {
          welcomeBannerImage: image,
          welcomeBannerTextColor: color,
          welcomeBannerLocation: location,
          welcomeBannerPageVisibility: visibility,
        });
        TEXTS.forEach(function (pair) {
          now[pair[0]] = bannerValue(form, pair[0]);
        });
        form.dataset.original = JSON.stringify(now);
        ui().toast(t("admin.config.welcome_banner.saved"));
      }, ui().failed);
  }

  // In the capture phase, before htmx's boosted form takes the submit.
  document.addEventListener(
    "submit",
    function (event) {
      var form = event.target;
      if (form.matches("form.admin-logo-form")) {
        event.preventDefault();
        event.stopImmediatePropagation();
        saveLogo(form);
      } else if (form.matches("form.admin-fonts-form")) {
        event.preventDefault();
        event.stopImmediatePropagation();
        submitFonts(form);
      } else if (form.matches("form.admin-welcome-banner-form")) {
        event.preventDefault();
        event.stopImmediatePropagation();
        saveBanner(form);
      }
    },
    true
  );

  Discourse.onPage(function () {
    if (logoForm() || fontsForm()) {
      ui().strings("admin-branding-strings");
    }
    var form = logoForm();
    if (form) {
      form.querySelectorAll(".file-uploader.has-image").forEach(lightbox);
    }
  });
})();
