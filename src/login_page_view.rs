//! The login, signup and login-required pages as the Ember app draws them:
//! templates/login.gjs (LoginPageController) with WelcomeHeader,
//! LocalLoginForm or CodeLoginForm's email step, LoginPageCta and
//! LoginButtons; templates/signup.gjs (SignupPageController) with
//! SignupProgressBar, the create-account form or CodeLoginForm in the
//! signup context, SignupPageCta; templates/discovery/login-required.gjs
//! with the static login page; templates/password-reset.gjs. Each in the
//! state the route enters it; static/js/login.js drives the rest (the
//! flash, logging in, the second factor, validations, the forgot password
//! modal).

use crate::topic_list_view::escape;

/// An enabled authenticator, as LoginMethod describes it to LoginButtons.
pub struct AuthButton {
    pub name: String,
    /// `title_override`, else `login.<name>.title`.
    pub title: String,
    /// `screenReaderTitle`: `title_override`, else `login.<name>.sr_title`
    /// falling back to the title.
    pub sr_title: String,
    /// The icon's name; Google draws its own (GoogleIcon).
    pub icon: String,
}

/// What every page here draws with.
pub struct Cx<'a> {
    /// `js.*` strings.
    pub t: &'a dyn Fn(&str) -> String,
    /// `site.mobileView`
    pub mobile: bool,
}

/// `dIcon`
pub fn icon(name: &str) -> String {
    crate::topic_list_view::icon(name, None)
}

/// DButton: `classes` are the invocation's (`class=`), `attrs` the other
/// attributes in the order DButton writes them (form, id, tabindex,
/// title), already escaped.
fn button(classes: &str, icon_name: Option<&str>, label: &str, attrs: &str) -> String {
    let (content, svg) = match icon_name {
        Some(name) if label.is_empty() => (" no-text btn-icon", icon(name)),
        Some(name) => (" btn-icon-text", icon(name)),
        None => ("", String::new()),
    };
    let label = if label.is_empty() {
        "<span aria-hidden=\"true\">&#8203;</span>".to_string()
    } else {
        format!("<span class=\"d-button-label\">{}</span>", escape(label))
    };
    format!(
        "<button class=\"btn{content} {classes}\"{attrs}>{svg}{label}</button>",
        attrs = if attrs.contains(" type=") {
            attrs.to_string()
        } else {
            format!("{attrs} type=\"button\"")
        }
    )
}

/// WelcomeHeader, with what its block yields.
fn welcome_header(title: &str, id: Option<&str>, inner: &str) -> String {
    let id = id.map(|i| format!(" id=\"{i}\"")).unwrap_or_default();
    format!(
        "<div class=\"login-welcome-header\"{id}><h1 class=\"login-title\">{}</h1>{inner}</div>",
        escape(title)
    )
}

/// DTogglePasswordMask, masked.
fn toggle_password_mask(cx: &Cx, tabindex: Option<&str>) -> String {
    let tabindex = tabindex
        .map(|t| format!(" tabindex=\"{t}\""))
        .unwrap_or_default();
    button(
        "btn-transparent toggle-password-mask",
        Some("far-eye"),
        "",
        &format!(
            "{tabindex} title=\"{}\"",
            escape(&(cx.t)("login.show_password_title"))
        ),
    )
}

/// The caps lock warning DPasswordField's owners draw, hidden.
fn caps_lock_warning(cx: &Cx) -> String {
    format!(
        "<div class=\"caps-lock-warning hidden\">{}{}</div>",
        icon("triangle-exclamation"),
        escape(&(cx.t)("login.caps_lock_warning"))
    )
}

/// LoginButtons in `context` ("login" or "create-account"): the
/// authenticators' buttons, then the passkey's on the login page.
pub fn login_buttons(cx: &Cx, buttons: &[AuthButton], passkeys: bool) -> String {
    let hidden = buttons.is_empty() && !passkeys;
    let mut classes = Vec::new();
    if hidden {
        classes.push("hidden");
    }
    if buttons.len() > 1 {
        classes.push("multiple");
    }
    let class = if classes.is_empty() {
        String::new()
    } else {
        format!(" class=\"{}\"", classes.join(" "))
    };
    let mut out = format!("<div{class} id=\"login-buttons\">");
    for b in buttons {
        let social = format!(
            "<button aria-label=\"{}\" class=\"btn btn-social {}\" type=\"button\">{}<span class=\"btn-social-title\">{}</span></button>",
            escape(&b.sr_title),
            escape(&b.name),
            if b.name == "google_oauth2" {
                GOOGLE_ICON.to_string()
            } else {
                icon(&b.icon)
            },
            escape(&b.title)
        );
        if b.name == "discourse_id" {
            out.push_str(&format!(
                "<div class=\"discourse-id__wrapper\">{social}<div class=\"btn-discourse-id__suffix\"><span class=\"btn-discourse-id__description\">{}</span>{}{}{}{}</div></div>",
                escape(&(cx.t)("login.works_with")),
                icon("fab-google"),
                icon("fab-apple"),
                icon("fab-facebook"),
                icon("fab-github"),
            ));
        } else {
            out.push_str(&social);
        }
    }
    if passkeys {
        out.push_str(&button(
            "btn-social passkey-login-button",
            Some("user"),
            &(cx.t)("login.passkey.name"),
            "",
        ));
    }
    out.push_str("</div>");
    out
}

/// GoogleIcon
const GOOGLE_ICON: &str = r##"<svg class="fa d-icon d-icon-custom-google-oauth2 svg-icon" viewBox="0 0 48 48"><defs><path d="M44.5 20H24v8.5h11.8C34.7 33.9 30.1 37 24 37c-7.2 0-13-5.8-13-13s5.8-13 13-13c3.1 0 5.9 1.1 8.1 2.9l6.4-6.4C34.6 4.1 29.6 2 24 2 11.8 2 2 11.8 2 24s9.8 22 22 22c11 0 21-8 21-22 0-1.3-.2-2.7-.5-4z" id="google-icon-a"></path></defs><clipPath id="google-icon-b"><use href="#google-icon-a" overflow="visible"></use></clipPath><path clip-path="url(#google-icon-b)" d="M0 37V11l17 13z" fill="#FBBC05"></path><path clip-path="url(#google-icon-b)" d="M0 11l17 13 7-6.1L48 14V0H0z" fill="#EA4335"></path><path clip-path="url(#google-icon-b)" d="M0 37l30-23 7.9 1L48 0v48H0z" fill="#34A853"></path><path clip-path="url(#google-icon-b)" d="M48 48L17 24l-4-3 35-10z" fill="#4285F4"></path></svg>"##;

/// CodeLoginForm's email step outside an invite: the email field, Continue,
/// and on the login page the way back to the password.
pub fn code_login_email_step(cx: &Cx, signup: bool, use_password: bool) -> String {
    let t = cx.t;
    let mut out = String::from("<div class=\"code-login-form\">");
    if signup {
        // heading: the email step's title in the signup context.
        out.push_str(&welcome_header(
            &t("code_login.signup_title"),
            Some("create-account-title"),
            "",
        ));
        out.push_str(&format!(
            "<p class=\"code-login-form__instructions\">{}</p>",
            escape(&t("code_login.signup_instructions"))
        ));
    }
    out.push_str(&format!(
        "<form class=\"form-kit code-login-form__email-step\" novalidate=\"\"><div class=\"form-kit__container form-kit__field form-kit__field-input-email --full\" data-name=\"email\" id=\"control-email\" data-control-type=\"input-email\"><label class=\"form-kit__container-title\" for=\"code-login-email\"><span>{}</span></label><div class=\"form-kit__container-content\"><div class=\"form-kit__control-input-wrapper\"><input name=\"email\" value=\"\" id=\"code-login-email\" class=\"form-kit__control-input\" autocomplete=\"username\" autofocus=\"autofocus\" type=\"email\"></div></div></div><div class=\"code-login-form__email-actions\">{}",
        escape(&t("code_login.email_label")),
        button(
            "btn-primary form-kit__button btn-primary code-login-form__continue",
            None,
            &t("code_login.continue_button"),
            " type=\"submit\"",
        ),
    ));
    if use_password {
        out.push_str(&button(
            "btn-flat code-login-form__password-toggle",
            None,
            &t("code_login.use_password_instead"),
            "",
        ));
    }
    out.push_str("</div></form></div>");
    out
}

/// The login page's state as the route enters it.
pub struct LoginView<'a> {
    pub cx: Cx<'a>,
    /// canLoginLocal: enable_local_logins.
    pub local: bool,
    /// canLoginLocalWithEmail: enable_local_logins_via_email.
    pub via_email: bool,
    /// canUseCodeLogin: login codes by email, with local logins.
    pub code_login: bool,
    /// showCodeLoginForm: `?mode=code` with code logins.
    pub code_mode: bool,
    /// canUsePasskeys
    pub passkeys: bool,
    pub buttons: &'a [AuthButton],
    /// application.canSignUp
    pub can_sign_up: bool,
}

/// LocalLoginForm before anything is typed.
fn local_login_form(v: &LoginView) -> String {
    let t = v.cx.t;
    let email_link = if !v.via_email {
        String::new()
    } else if v.code_login {
        format!(
            "<a href=\"\" id=\"one-time-code-link\" tabindex=\"3\">{}</a>",
            escape(&t("code_login.email_me_code"))
        )
    } else {
        format!(
            "<a class=\"no-login-filled\" href=\"\" id=\"email-login-link\" tabindex=\"3\">{}</a>",
            escape(&t("email_login.login_link"))
        )
    };
    format!(
        "<form id=\"login-form\" method=\"post\" novalidate=\"\"><div class=\"\" id=\"credentials\">\
         <div class=\"input-group\"><input id=\"login-account-name\" class=\"ember-text-field ember-view\" autocapitalize=\"off\" autocomplete=\"{}\" autocorrect=\"off\" autofocus=\"autofocus\" inputmode=\"email\" tabindex=\"1\" value=\"\" type=\"text\"><label class=\"alt-placeholder\" for=\"login-account-name\">{}</label>{email_link}</div>\
         <div class=\"input-group\"><input placeholder=\"\" tabindex=\"1\" maxlength=\"200\" value=\"\" autocomplete=\"current-password\" id=\"login-account-password\" class=\"ember-text-field ember-view\" type=\"password\"><label class=\"alt-placeholder\" for=\"login-account-password\">{}</label><div class=\"login__password-links\"><a href=\"\" id=\"forgot-password-link\" tabindex=\"2\">{}</a></div>{}</div>\
         </div></form>",
        if v.passkeys {
            "username webauthn"
        } else {
            "username"
        },
        escape(&t("login.email_placeholder")),
        escape(&t("login.password")),
        escape(&t("forgot_password.action")),
        caps_lock_warning(&v.cx),
    )
}

/// LoginPageCta
fn login_page_cta(v: &LoginView) -> String {
    let t = v.cx.t;
    let mut out =
        String::from("<div class=\"login-page-cta\"><div class=\"login-page-cta__buttons\">");
    if v.local {
        out.push_str(&button(
            "btn-large btn-primary login-page-cta__login",
            None,
            &t("login.title"),
            " form=\"login-form\" id=\"login-button\" tabindex=\"2\" type=\"submit\"",
        ));
        if v.can_sign_up {
            out.push_str(&format!(
                "<span class=\"login-page-cta__no-account-yet\">{}</span>",
                escape(&t("create_account.no_account_yet"))
            ));
            out.push_str(&button(
                "btn-large btn-flat login-page-cta__signup",
                None,
                &t("create_account.title"),
                " id=\"new-account-link\" tabindex=\"3\"",
            ));
        }
    }
    out.push_str("</div></div>");
    out
}

/// NoLoginMethods
fn no_login_methods(cx: &Cx, base_path: &str) -> String {
    format!(
        "<div class=\"login-welcome-header no-login-methods-configured\"><h1 class=\"login-title\">{}</h1><img><p class=\"login-subheader\">{}</p></div>",
        escape(&(cx.t)("login.no_login_methods.title")),
        (cx.t)("login.no_login_methods.description")
            .replace("%{adminLoginPath}", &format!("{base_path}/u/admin-login"))
    )
}

/// templates/login.gjs
pub fn login_page(v: &LoginView, base_path: &str) -> String {
    let t = v.cx.t;
    let desktop = !v.cx.mobile;
    let has_button = !v.buttons.is_empty() || v.passkeys;
    let no_options = !has_button && !v.local;
    let code = v.code_login && v.code_mode;
    // bodyClasses, after the "login-body" dConcatClass puts first.
    let mut classes = vec!["login-body", "login-body"];
    if has_button {
        classes.push("has-alt-auth");
    }
    if !v.local {
        classes.push("no-local-login");
    }
    if code {
        classes.push("code-login");
    }
    let left = if desktop {
        "<div class=\"login-left-side\">"
    } else {
        "<div>"
    };
    let header = || welcome_header(&t("login.header_title"), None, "");
    let mut out = format!(
        // hx-boost off: the forms here are the script's, not page loads.
        "<div class=\"login-fullpage\" hx-boost=\"false\"><div class=\"{}\">",
        classes.join(" ")
    );
    if no_options {
        out.push_str(left);
        out.push_str(&no_login_methods(&v.cx, base_path));
        out.push_str("</div>");
    } else {
        if !desktop {
            out.push_str(&header());
            out.push_str(&login_buttons(&v.cx, v.buttons, v.passkeys));
        }
        if v.local {
            out.push_str(left);
            if desktop {
                out.push_str(&header());
            }
            if code {
                out.push_str(&code_login_email_step(&v.cx, false, true));
            } else {
                out.push_str(&local_login_form(v));
            }
            if desktop && !code {
                out.push_str(&login_page_cta(v));
            }
            out.push_str("</div>");
        }
        if desktop {
            if !v.local {
                out.push_str("<div class=\"login-left-side\">");
                out.push_str(&header());
                out.push_str("</div>");
            }
            if has_button {
                out.push_str("<div class=\"login-right-side\">");
                out.push_str(&login_buttons(&v.cx, v.buttons, v.passkeys));
                out.push_str("</div>");
            }
        }
    }
    if !desktop && !code && !no_options {
        out.push_str(&login_page_cta(v));
    }
    out.push_str("</div></div>");
    // The forms the one-time code link and its way back swap in
    // (codeLoginSelected), for static/js/login.js.
    if v.code_login && v.local {
        out.push_str(&format!(
            "<template id=\"login-code-form\">{}</template><template id=\"login-password-form\">{}</template><template id=\"login-cta\">{}</template>",
            code_login_email_step(&v.cx, false, true),
            local_login_form(v),
            login_page_cta(v)
        ));
    }
    out
}

/// templates/discovery/login-required.gjs with the static login page
/// (static/login.html.erb) as `content`.
pub fn login_required_page(cx: &Cx, content: &str, can_sign_up: bool) -> String {
    let t = cx.t;
    let mut buttons = String::new();
    if can_sign_up {
        buttons.push_str(&button(
            "btn-primary sign-up-button",
            None,
            &t("sign_up"),
            "",
        ));
    }
    buttons.push_str(&button(
        "btn-primary login-button",
        Some("user"),
        &t("log_in"),
        "",
    ));
    format!(
        "<section class=\"container\"><div class=\"contents clearfix body-page\"><div class=\"login-welcome\"><div class=\"login-content\">{content}</div><div class=\"body-page-button-container\">{buttons}</div></div></div></section>"
    )
}

/// static/login.html.erb: the logo (its dark source when there is one) or
/// a waving hand, the cooked welcome message and the site's description.
pub fn static_login_content(
    logo: Option<(&str, Option<&str>)>,
    waving_hand: &str,
    site_title: &str,
    welcome_cooked: &str,
    description: &str,
) -> String {
    let picture = match logo {
        Some((url, dark)) => format!(
            "<picture class=\"logo-container\">{}<img src=\"{}\" alt=\"{}\" class=\"site-logo\"></picture>",
            dark.map(|d| format!(
                "<source srcset=\"{}\" media=\"(prefers-color-scheme: dark)\">",
                escape(d)
            ))
            .unwrap_or_default(),
            escape(url),
            escape(site_title)
        ),
        None => format!(
            "<img src=\"{}\" alt=\"\" class=\"waving-hand\">",
            escape(waving_hand)
        ),
    };
    format!(
        "{picture}<div class=\"login-welcome__title\">{welcome_cooked}<p class=\"login-welcome__description\">{}</p></div>",
        escape(description)
    )
}

/// The signup page's state as the route enters it (no authentication
/// data: static/js/login.js redraws the form for an external login's).
pub struct SignupView<'a> {
    pub cx: Cx<'a>,
    /// canCreateLocal: enable_local_logins.
    pub local: bool,
    /// showCodeSignupForm: login codes by email, with local logins.
    pub code_signup: bool,
    pub buttons: &'a [AuthButton],
    /// must_approve_users: the progress bar's approve step.
    pub must_approve: bool,
    pub full_name_visible: bool,
    pub full_name_required: bool,
    pub require_invite_code: bool,
    pub show_email_instructions: bool,
    pub show_username_instructions: bool,
    pub show_password_instructions: bool,
    pub show_full_name_instructions: bool,
    pub min_password_length: i64,
    /// create_account.disclaimer, when the site has both a ToS and a
    /// privacy policy.
    pub disclaimer: Option<String>,
}

/// SignupProgressBar at `step`.
pub fn progress_bar(must_approve: bool, step: &str) -> String {
    let steps: &[&str] = if must_approve {
        &["signup", "activate", "approve", "login"]
    } else {
        &["signup", "activate", "login"]
    };
    let current = steps.iter().position(|s| *s == step).unwrap_or(0);
    let mut out = String::from("<div class=\"signup-progress-bar\">");
    for (i, _) in steps.iter().enumerate() {
        let state = match i.cmp(&current) {
            std::cmp::Ordering::Equal => "active",
            std::cmp::Ordering::Less => "completed",
            std::cmp::Ordering::Greater => "incomplete",
        };
        out.push_str(&format!(
            "<div class=\"signup-progress-bar__segment --{state}\"><div class=\"signup-progress-bar__step\"><div class=\"signup-progress-bar__circle\"></div></div></div>"
        ));
    }
    out.push_str("</div>");
    out
}

/// FullnameInput, empty.
fn fullname_input(v: &SignupView, required: bool) -> String {
    let t = v.cx.t;
    let title = if v.full_name_required {
        t("user.name.title")
    } else {
        t("user.name.title_optional")
    };
    let tip = if v.show_full_name_instructions {
        format!(
            "<span class=\"more-info\" id=\"fullname-validation-more-info\">{}</span>",
            escape(&t("user.name.instructions_required"))
        )
    } else {
        "<div class=\"tip good\" id=\"fullname-validation\"></div>".to_string()
    };
    format!(
        "<div class=\"input-group create-account__fullname{}\"><input id=\"new-account-name\" class=\"ember-text-field ember-view\" aria-describedby=\"fullname-validation fullname-validation-more-info\"{} name=\"name\" type=\"text\"><label class=\"alt-placeholder\" for=\"new-account-name\">{}</label>{tip}</div>",
        if required { " required" } else { "" },
        if v.full_name_required {
            " aria-invalid=\"\""
        } else {
            ""
        },
        escape(&title),
    )
}

/// The create-account form (`showCreateForm`) and SignupPageCta, empty.
pub fn create_account_form(v: &SignupView) -> String {
    let t = v.cx.t;
    let mut out = String::from("<form id=\"login-form\">");
    out.push_str(&format!(
        "<div class=\"input-group create-account-email\"><input id=\"new-account-email\" class=\"ember-text-field ember-view\" aria-describedby=\"account-email-validation account-email-validation-more-info\" aria-invalid=\"\" autofocus=\"autofocus\" name=\"email\" type=\"email\"><label class=\"alt-placeholder\" for=\"new-account-email\">{}</label><span class=\"more-info\" id=\"account-email-validation-more-info\">{}</span></div>",
        escape(&t("user.email.title")),
        if v.show_email_instructions {
            escape(&t("user.email.instructions"))
        } else {
            String::new()
        },
    ));
    out.push_str(&format!(
        "<div class=\"input-group create-account__username\"><input aria-describedby=\"username-validation username-validation-more-info\" aria-invalid=\"\" autocomplete=\"off\" class=\"\" id=\"new-account-username\" name=\"username\" type=\"text\"><label class=\"alt-placeholder\" for=\"new-account-username\">{}</label>{}</div>",
        escape(&t("user.username.title")),
        if v.show_username_instructions {
            format!(
                "<span class=\"more-info\" id=\"username-validation-more-info\">{}</span>",
                escape(&t("user.username.instructions"))
            )
        } else {
            "<div class=\"tip bad\" id=\"username-validation\"></div>".to_string()
        }
    ));
    if v.full_name_visible && v.full_name_required {
        out.push_str(&fullname_input(v, true));
    }
    out.push_str(&format!(
        "<div class=\"input-group create-account__password\"><input placeholder=\"\" aria-describedby=\"password-validation password-validation-more-info\" aria-invalid=\"\" autocomplete=\"current-password\" id=\"new-account-password\" class=\"ember-text-field ember-view\" type=\"password\"><label class=\"alt-placeholder\" for=\"new-account-password\">{}</label>{}<div class=\"create-account__password-info\"><div class=\"create-account__password-tip-validation\">{}{}</div></div><div class=\"password-confirmation\"><label for=\"new-account-password-confirmation\">{}</label><input placeholder=\"\" autocomplete=\"new-password\" id=\"new-account-confirmation\" class=\"ember-text-field ember-view\" type=\"text\"><input id=\"new-account-challenge\" class=\"ember-text-field ember-view\" type=\"text\"></div></div>",
        escape(&t("user.password.title")),
        toggle_password_mask(&v.cx, None),
        if v.show_password_instructions {
            format!(
                "<span class=\"more-info\" id=\"password-validation-more-info\">{}</span>",
                escape(
                    &t("user.password.instructions")
                        .replace("%{count}", &v.min_password_length.to_string())
                )
            )
        } else {
            String::new()
        },
        caps_lock_warning(&v.cx),
        escape(&t("user.password_confirmation.title")),
    ));
    if v.require_invite_code {
        out.push_str(&format!(
            "<div class=\"input-group create-account__invite-code\"><input id=\"inviteCode\" class=\"ember-text-field ember-view\" type=\"text\"><label class=\"alt-placeholder\" for=\"invite-code\">{}</label><span class=\"more-info\">{}</span></div>",
            escape(&t("user.invite_code.title")),
            escape(&t("user.invite_code.instructions")),
        ));
    }
    if v.full_name_visible && !v.full_name_required {
        out.push_str(&fullname_input(v, false));
    }
    out.push_str("</form>");
    // SignupPageCta
    out.push_str("<div class=\"signup-page-cta\">");
    if let Some(disclaimer) = &v.disclaimer {
        out.push_str(&format!(
            "<div class=\"signup-page-cta__disclaimer\">{disclaimer}</div>"
        ));
    }
    out.push_str("<div class=\"signup-page-cta__buttons\">");
    out.push_str(&button(
        "btn-large btn-primary signup-page-cta__signup",
        None,
        &t("create_account.title"),
        "",
    ));
    out.push_str(&format!(
        "<span class=\"signup-page-cta__existing-account\">{}</span>",
        escape(&t("create_account.already_have_account"))
    ));
    out.push_str(&button(
        "btn-large btn-flat signup-page-cta__login",
        None,
        &t("log_in"),
        "",
    ));
    out.push_str("</div></div>");
    out
}

/// templates/signup.gjs, and the create-account form an external login's
/// data needs (in a template when the page shows the code signup).
pub fn signup_page(v: &SignupView, base_path: &str) -> String {
    let t = v.cx.t;
    let desktop = !v.cx.mobile;
    let has_button = !v.buttons.is_empty();
    let no_options = !has_button && !v.local;
    let mut classes = vec!["signup-body", "has-user-fields"];
    if has_button {
        classes.push("has-alt-auth");
    }
    if !v.local {
        classes.push("no-local-logins");
    }
    if v.code_signup {
        classes.push("passwordless-signup");
    }
    let mut out = format!(
        "<div class=\"signup-fullpage\" hx-boost=\"false\"><div class=\"{}\">{}",
        classes.join(" "),
        if desktop {
            "<div class=\"login-left-side\">"
        } else {
            "<div>"
        }
    );
    let header = format!(
        "{}{}",
        progress_bar(v.must_approve, "signup"),
        welcome_header(
            &t("create_account.header_title"),
            Some("create-account-title"),
            ""
        )
    );
    if no_options {
        out.push_str(&no_login_methods(&v.cx, base_path));
    } else if !v.code_signup {
        out.push_str(&header);
    }
    if v.code_signup {
        out.push_str(&code_login_email_step(&v.cx, true, false));
        if let Some(disclaimer) = &v.disclaimer {
            out.push_str(&format!(
                "<div class=\"signup-page-cta__disclaimer\">{disclaimer}</div>"
            ));
        }
    } else if v.local {
        out.push_str(&create_account_form(v));
    }
    out.push_str("</div>");
    if has_button {
        if !desktop {
            out.push_str(&format!(
                "<div class=\"login-or-separator\"><span>{}</span></div>",
                escape(&t("login.or"))
            ));
        }
        out.push_str("<div class=\"login-right-side\">");
        out.push_str(&login_buttons(&v.cx, v.buttons, false));
        out.push_str("</div>");
    }
    out.push_str("</div></div>");
    // An external login's new account: the progress bar, header and form
    // the page shows with its data (showCreateForm with authOptions).
    if v.code_signup || !v.local {
        out.push_str(&format!(
            "<template id=\"signup-auth-form\">{header}{}</template>",
            create_account_form(v)
        ));
    }
    out
}

/// ActivationControls: resend unless accounts wait for approval, and a
/// change of address where logins or emails allow one.
pub fn activation_controls(cx: &Cx, must_approve: bool, can_edit_email: bool) -> String {
    let mut out = String::from("<div class=\"activation-controls\">");
    if !must_approve {
        out.push_str(&button(
            "btn-primary resend",
            Some("envelope"),
            &(cx.t)("login.resend_title"),
            "",
        ));
    }
    if can_edit_email {
        out.push_str(&button(
            "btn-default edit-email",
            Some("pencil"),
            &(cx.t)("login.change_email"),
            "",
        ));
    }
    out.push_str("</div>");
    out
}

/// templates/account-created.gjs with account-created/index.gjs: the
/// progress bar at activate, the welcome, the signup's message (HTML,
/// as the server wrote it) and the activation controls.
pub fn account_created_page(
    cx: &Cx,
    site_title: &str,
    message: &str,
    show_controls: bool,
    must_approve: bool,
    can_edit_email: bool,
) -> String {
    let mut out = format!(
        "<div class=\"account-created\">{}{}<div class=\"success-info\">{message}</div>",
        progress_bar(must_approve, "activate"),
        welcome_header(
            &(cx.t)("invites.welcome_to").replace("%{site_name}", site_title),
            None,
            ""
        ),
    );
    if show_controls {
        out.push_str(&activation_controls(cx, must_approve, can_edit_email));
    }
    out.push_str("</div>");
    out
}

/// The second factors of the user a reset link is for.
pub struct ResetView {
    /// second_factor_required: TOTP.
    pub totp: bool,
    /// security_key_required
    pub security_key: bool,
    pub backup: bool,
    /// multiple_second_factor_methods
    pub other_method: bool,
}

/// SecondFactorForm (`@isLogin` false) around DSecondFactorInput with the
/// id `input_id`, for `method` (1 TOTP, 2 backup code).
pub fn second_factor_form(cx: &Cx, method: u8, totp: bool, backup: bool, input_id: &str) -> String {
    let t = cx.t;
    let (title, description) = if method == 2 {
        (
            t("login.second_factor_backup_title"),
            t("login.second_factor_backup_description"),
        )
    } else {
        (
            t("login.second_factor_title"),
            t("login.second_factor_description"),
        )
    };
    let input = if method == 1 {
        // DOtp, its attributes on the input.
        let mut slots = String::new();
        for i in 0..6 {
            slots.push_str(&format!(
                "<div class=\"d-otp-slot{}\" data-index=\"{i}\"><span aria-hidden=\"true\">&#8203;</span>{}</div>",
                if i == 0 {
                    " --is-focused --show-cursor"
                } else {
                    " --placeholder"
                },
                if i == 0 { "" } else { "-" }
            ));
        }
        format!(
            "<div class=\"d-otp\"><div class=\"d-otp-group\">{slots}</div><div class=\"d-otp-input-wrapper\"><input aria-label=\"{}\" autocapitalize=\"off\" autocomplete=\"one-time-code\" autocorrect=\"off\" autofocus=\"\" class=\"d-otp-input second-factor-token-input\" data-slot=\"input-otp\" inputmode=\"numeric\" maxlength=\"6\" spellcheck=\"false\" id=\"{input_id}\"></div></div>",
            escape(&t("d_otp.screen_reader.other").replace("%{count}", "6"))
        )
    } else {
        format!(
            "<input autocapitalize=\"off\" autocorrect=\"off\" autofocus=\"autofocus\" class=\"second-factor-token-input\" maxlength=\"32\" pattern=\"[a-z0-9]{{16}}\" type=\"text\" id=\"{input_id}\">"
        )
    };
    let toggle = if backup && totp {
        format!(
            "<p><a class=\"toggle-second-factor-method\" href=\"\">{}</a></p>",
            escape(&t(if method == 1 {
                "user.second_factor_backup.use"
            } else {
                "user.second_factor.use"
            }))
        )
    } else {
        String::new()
    };
    format!(
        "<div id=\"second-factor\"><h3>{}</h3><p class=\"second-factor__description\">{}</p>{input}{toggle}</div>",
        escape(&title),
        escape(&description)
    )
}

/// SecurityKeyForm with the single security key button.
pub fn security_key_form(cx: &Cx, other_method: bool) -> String {
    format!(
        "<div id=\"security-key\">{}<p>{}</p></div>",
        button(
            "btn-large btn-primary",
            Some("key"),
            &(cx.t)("login.security_key_authenticate"),
            " id=\"security-key-authenticate-button\"",
        ),
        if other_method {
            format!(
                "<a class=\"toggle-second-factor-method\" href=\"\">{}</a>",
                escape(&(cx.t)("login.security_key_alternative"))
            )
        } else {
            String::new()
        }
    )
}

/// The new password's field and Set Password (password-reset.gjs).
pub fn new_password_form(cx: &Cx) -> String {
    let t = cx.t;
    format!(
        "<h2>{}</h2><div class=\"input\"><input placeholder=\"\" autocomplete=\"new-password\" autofocus=\"autofocus\" id=\"new-account-password\" class=\"ember-text-field ember-view change-password-form__password\" type=\"password\"><div class=\"change-password__password-info\"><div class=\"change-password_tip-validation\">{}</div>{}</div></div>{}",
        escape(&t("user.change_password.choose_new")),
        caps_lock_warning(cx),
        toggle_password_mask(cx, None),
        button(
            "btn-primary",
            None,
            &t("user.change_password.set_password"),
            " type=\"submit\""
        ),
    )
}

/// templates/password-reset.gjs as the route enters it: the second
/// factor first when the user has one (initSelectedSecondFactorMethod),
/// else the new password.
pub fn password_reset_page(cx: &Cx, v: &ResetView) -> String {
    let t = cx.t;
    let mut out = String::from(
        "<div class=\"container password-reset clearfix\" hx-boost=\"false\"><form class=\"change-password-form login-left-side\">",
    );
    if v.totp || v.security_key {
        out.push_str(&format!(
            "<h2>{}</h2><p>{}</p>",
            escape(&t("user.change_password.title")),
            escape(&t("user.change_password.verify_identity"))
        ));
        if v.security_key {
            out.push_str(&security_key_form(cx, v.other_method));
        } else {
            out.push_str(&second_factor_form(
                cx,
                1,
                v.totp,
                v.backup,
                "second-factor",
            ));
            out.push_str(&button(
                "btn-primary",
                None,
                &t("submit"),
                " type=\"submit\"",
            ));
        }
    } else {
        out.push_str(&new_password_form(cx));
    }
    out.push_str("</form></div>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(key: &str) -> String {
        format!("[{key}]")
    }

    fn view<'a>(buttons: &'a [AuthButton]) -> LoginView<'a> {
        LoginView {
            cx: Cx {
                t: &t,
                mobile: false,
            },
            local: true,
            via_email: true,
            code_login: false,
            code_mode: false,
            passkeys: true,
            buttons,
            can_sign_up: true,
        }
    }

    #[test]
    fn login_page_desktop_has_both_sides() {
        let html = login_page(&view(&[]), "");
        assert!(html.starts_with("<div class=\"login-fullpage\" hx-boost=\"false\"><div class=\"login-body login-body has-alt-auth\"><div class=\"login-left-side\"><div class=\"login-welcome-header\">"), "{html}");
        assert!(html.contains("id=\"email-login-link\""), "{html}");
        assert!(html.contains("<div class=\"login-right-side\"><div id=\"login-buttons\"><button class=\"btn btn-icon-text btn-social passkey-login-button\" type=\"button\">"), "{html}");
        assert!(html.contains("<button class=\"btn btn-large btn-primary login-page-cta__login\" form=\"login-form\" id=\"login-button\" tabindex=\"2\" type=\"submit\"><span class=\"d-button-label\">[login.title]</span></button>"), "{html}");
    }

    #[test]
    fn login_page_mobile_puts_the_buttons_first() {
        let mut v = view(&[]);
        v.cx.mobile = true;
        let html = login_page(&v, "");
        let buttons = html.find("id=\"login-buttons\"").unwrap();
        let form = html.find("id=\"login-form\"").unwrap();
        assert!(buttons < form, "{html}");
        assert!(!html.contains("login-left-side"), "{html}");
        assert!(html.ends_with("</div></div></div></div>"), "{html}");
    }

    #[test]
    fn no_login_methods() {
        let mut v = view(&[]);
        v.local = false;
        v.passkeys = false;
        let html = login_page(&v, "/forum");
        assert!(html.contains("no-login-methods-configured"), "{html}");
        assert!(html.contains("no-local-login"), "{html}");
        assert!(!html.contains("login-page-cta"), "{html}");
    }

    #[test]
    fn code_signup_keeps_a_create_form_for_external_logins() {
        let v = SignupView {
            cx: Cx {
                t: &t,
                mobile: false,
            },
            local: true,
            code_signup: true,
            buttons: &[],
            must_approve: false,
            full_name_visible: false,
            full_name_required: false,
            require_invite_code: false,
            show_email_instructions: true,
            show_username_instructions: false,
            show_password_instructions: false,
            show_full_name_instructions: false,
            min_password_length: 10,
            disclaimer: None,
        };
        let html = signup_page(&v, "");
        assert!(html.contains("<div class=\"signup-body has-user-fields passwordless-signup\"><div class=\"login-left-side\"><div class=\"code-login-form\"><div class=\"login-welcome-header\" id=\"create-account-title\">"), "{html}");
        assert!(
            html.contains("<template id=\"signup-auth-form\"><div class=\"signup-progress-bar\">"),
            "{html}"
        );
    }
}
