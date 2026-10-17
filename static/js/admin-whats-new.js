// What's new (DashboardNewFeatures): each month's heading put inside the
// card's title as Glimmer builds it (the parser keeps an h2 out of an h3),
// and Check for updates, which loads the feed again
// (/admin/whats-new.json?force_refresh=true) behind the spinner and draws
// the page anew, or on failure the error's empty list and popupAjaxError.
(function () {
  "use strict";

  if (!Discourse.once("admin-whats-new")) {
    return;
  }
  var ui = Discourse.adminUi;
  ui.strings("admin-whats-new-strings");

  // The months in the browser's zone, as moment(released_at || created_at)
  // keys them (the server grouped in UTC): the cards drawn again when an
  // item belongs to another month here, labelled as moment formats the
  // month ("MMMM YYYY").
  var datesScript = document.getElementById("admin-whats-new-dates");
  var holder = document.querySelector(".admin-config-area__primary-content .loading-container");
  if (datesScript && holder) {
    var dates = JSON.parse(datesScript.textContent);
    var items = Array.prototype.slice.call(
      holder.querySelectorAll(".admin-new-features__group .admin-new-feature-item")
    );
    var lang = document.documentElement.lang || "en";
    var groups = [];
    items.forEach(function (item, i) {
      var d = new Date(dates[i]);
      var key = d.getFullYear() + "-" + d.getMonth();
      var group = groups.find(function (g) {
        return g.key === key;
      });
      if (!group) {
        group = {
          key: key,
          label: new Date(d.getFullYear(), d.getMonth(), 1).toLocaleString(lang, {
            month: "long",
            year: "numeric",
          }),
          items: [],
        };
        groups.push(group);
      }
      group.items.push(item);
    });
    var drawn = Array.prototype.map.call(
      holder.querySelectorAll(".admin-new-features__group"),
      function (section) {
        return section.dataset.newFeaturesGroup + ":" + section.querySelectorAll(".admin-new-feature-item").length;
      }
    );
    var wanted = groups.map(function (g) {
      return g.label + ":" + g.items.length;
    });
    if (items.length === dates.length && drawn.join("|") !== wanted.join("|")) {
      holder.replaceChildren.apply(
        holder,
        groups.map(function (g) {
          var section = document.createElement("section");
          section.className = "admin-config-area-card admin-new-features__group";
          section.setAttribute("data-new-features-group", g.label);
          section.innerHTML =
            '<div class="admin-config-area-card__header-wrapper"><h3 class="admin-config-area-card__title"></h3><h2></h2></div><div class="admin-config-area-card__content"></div>';
          section.querySelector("h2").textContent = g.label;
          var content = section.querySelector(".admin-config-area-card__content");
          g.items.forEach(function (item) {
            content.appendChild(item);
          });
          return section;
        })
      );
    }
  }

  document.querySelectorAll(".admin-new-features__group").forEach(function (group) {
    var title = group.querySelector(".admin-config-area-card__title");
    var heading = group.querySelector(".admin-config-area-card__header-wrapper > h2");
    if (title && heading) {
      title.appendChild(heading);
    }
  });

  // ?scrollTo=<change>: its card scrolled to and highlighted for two
  // seconds (an automatically enabled change that has become permanent).
  var scrollTo = new URLSearchParams(window.location.search).get("scrollTo");
  if (scrollTo) {
    var target = document.getElementById("upcoming-change-" + scrollTo);
    if (target) {
      target.scrollIntoView({ block: "center", behavior: "smooth" });
      target.classList.add("--highlighted");
      setTimeout(function () {
        target.classList.remove("--highlighted");
      }, 2000);
    }
  }

  document.addEventListener("click", function (event) {
    if (!event.target.closest(".admin-whats-new__check")) {
      return;
    }
    var holder = document.querySelector(".admin-config-area__primary-content .loading-container");
    if (!holder) {
      return;
    }
    holder.classList.add("visible");
    holder.setAttribute("data-loading", "true");
    holder.innerHTML = '<div class="spinner "></div>';
    ui.ajax("GET", "/admin/whats-new.json?force_refresh=true").then(
      function () {
        window.location.reload();
      },
      function (error) {
        holder.classList.remove("visible");
        holder.removeAttribute("data-loading");
        holder.innerHTML =
          '<div class="admin-config-area-empty-list"><span class="admin-config-area-empty-list__title"></span></div>';
        holder.querySelector(".admin-config-area-empty-list__title").innerHTML = ui.t(
          "admin.dashboard.new_features.no_new_features_error",
          { url: "https://releases.discourse.org/" }
        );
        ui.failed(error);
      }
    );
  });
})();
