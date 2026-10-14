//! DISCOURSE_ONEBOX against Ruby: each expected string is what
//! `Sanitize.fragment(html, Onebox::SanitizeConfig::DISCOURSE_ONEBOX
//! .merge(allowed_iframe_regexes: ...))` returned (sanitize 7.0.0,
//! Nokogiri 1.19.4). The template cases are lib/onebox/templates
//! rendered with sample data inside `_layout.mustache`.

use regex::Regex;

use super::clean_element::frag;
use crate::sanitize::onebox_config;

fn check(input: &str, regexes: Option<&[&str]>, expected: &str) {
    let regexes = regexes.map(|r| {
        r.iter()
            .map(|s| Regex::new(s).expect("valid regex"))
            .collect()
    });
    assert_eq!(frag(input, &onebox_config(regexes)), expected);
}

#[test]
fn template_allowlistedgeneric() {
    check(
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt=""/>

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener" title="10:00">example.com &ndash; Oct 1</a>
  </header>

  <article class="onebox-body">
    <img width="100" height="50" src="https://example.com/img.png" class="thumbnail" alt=""/>

<h3><a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener">A &quot;title&quot; &lt;with&gt; &amp; stuff</a></h3>

  <p>Some description here</p>
  
  <p>
    <span class="label1">l1: d1</span>
      <span class="label2">l2: d2</span>
  </p>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
        None,
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt="">

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener" title="10:00">example.com – Oct 1</a>
  </header>

  <article class="onebox-body">
    <img width="100" height="50" src="https://example.com/img.png" class="thumbnail" alt="">

<h3><a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener">A "title" &lt;with&gt; &amp; stuff</a></h3>

  <p>Some description&nbsp;here</p>
  
  <p>
    <span class="label1">l1: d1</span>
      <span class="label2">l2: d2</span>
  </p>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
    );
}

#[test]
fn template_amazon() {
    check(
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt=""/>

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener" title="10:00">example.com &ndash; Oct 1</a>
  </header>

  <article class="onebox-body">
    <img src="https://example.com/img.png" class="thumbnail">

<h3><a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener">A "title" <with> & stuff</a></h3>

<b>by someone</b>

<p>Some description here</p>

<p>
  4.5
  ISBN: 123, 
  Pub, 
  2020
  <strong>$12.00</strong>
</p>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
        None,
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt="">

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener" title="10:00">example.com – Oct 1</a>
  </header>

  <article class="onebox-body">
    <img src="https://example.com/img.png" class="thumbnail">

<h3><a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener">A "title"  &amp; stuff</a></h3>

<b>by someone</b>

<p>Some description&nbsp;here</p>

<p>
  4.5
  ISBN: 123, 
  Pub, 
  2020
  <strong>$12.00</strong>
</p>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
    );
}

#[test]
fn template_githubrepo() {
    check(
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt=""/>

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener" title="10:00">example.com &ndash; Oct 1</a>
  </header>

  <article class="onebox-body">
    <div class="github-row" data-github-private-repo="false">
  <img width="695" height="347" src="https://example.com/t.png" class="thumbnail">

  <h3><a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener ugc noopener">A &quot;title&quot; &lt;with&gt; &amp; stuff</a></h3>

    <p><span class="github-repo-description">Some description here</span></p>
</div>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
        None,
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt="">

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener" title="10:00">example.com – Oct 1</a>
  </header>

  <article class="onebox-body">
    <div class="github-row" data-github-private-repo="false">
  <img width="695" height="347" src="https://example.com/t.png" class="thumbnail">

  <h3><a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener">A "title" &lt;with&gt; &amp; stuff</a></h3>

    <p><span class="github-repo-description">Some description&nbsp;here</span></p>
</div>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
    );
}

#[test]
fn template_githubblob() {
    check(
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt=""/>

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener" title="10:00">example.com &ndash; Oct 1</a>
  </header>

  <article class="onebox-body">
    <h4><a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener">A &quot;title&quot; &lt;with&gt; &amp; stuff</a></h4>

<div class="git-blob-info">
  <a href="https://example.com/a?b=1&amp;c=2" rel="noopener"><code>abc123</code></a>
</div>


      <pre><code class="ruby">
line1
line2 &lt;tag&gt;</code></pre>



  Truncated <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener">Show original</a>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
        None,
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt="">

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener" title="10:00">example.com – Oct 1</a>
  </header>

  <article class="onebox-body">
    <h4><a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener">A "title" &lt;with&gt; &amp; stuff</a></h4>

<div class="git-blob-info">
  <a href="https://example.com/a?b=1&amp;c=2" rel="nofollow ugc noopener"><code>abc123</code></a>
</div>


      <pre><code class="ruby">
line1
line2 &lt;tag&gt;</code></pre>



  Truncated <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener">Show original</a>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
    );
}

#[test]
fn template_twitterstatus() {
    check(
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt=""/>

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener" title="10:00">example.com &ndash; Oct 1</a>
  </header>

  <article class="onebox-body">
    <img src="https://example.com/av.png" class="thumbnail onebox-avatar" alt="">
<h4><a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener">A &quot;title&quot; &lt;with&gt; &amp; stuff</a></h4>
<div class="twitter-screen-name"><a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener">@user</a></div>

<div class="tweet">
    <span class="is-reply">
      <svg class="fa d-icon d-icon-reply svg-icon svg-string" xmlns="http://www.w3.org/2000/svg"><use href="#reply"></use></svg>
    </span>
  <span class="tweet-description">Hello <b>world</b> <a href='https://t.co/x' target='_blank'>link</a> <script>x()</script></span>
    <div class="quoted">
      <a class="quoted-link" href="https://x.com/q">
        <p class="quoted-title">Q <span>@qq</span></p>
      </a>

      <div>quoted</div>
    </div>
</div>

<div class="date">
  <a href="https://example.com/a?b=1&amp;c=2" class="timestamp" target="_blank" rel="noopener">1:00 PM</a>

    <span class="like">
      <svg viewBox="0 0 512 512" width="14px" height="16px" aria-hidden="true">
        <path d="M462.3 62.6C407.5 15.9 326 24.3 275.7 76.2L256 96.5l-19.7-20.3C186.1 24.3 104.5 15.9 49.7 62.6c-62.8 53.6-66.1 149.8-9.9 207.9l193.5 199.8c12.5 12.9 32.8 12.9 45.3 0l193.5-199.8c56.3-58.1 53-154.3-9.8-207.9z"></path>
      </svg>
      3
    </span>

    <span class="retweet">
      <svg viewBox="0 0 640 512" width="14px" height="16px" aria-hidden="true">
        <path d="M629.657 343.598L528.971 444.284c-9.373 9.372-24.568 9.372-33.941 0L394.343 343.598c-9.373-9.373-9.373-24.569 0-33.941l10.823-10.823c9.562-9.562 25.133-9.34 34.419.492L480 342.118V160H292.451a24.005 24.005 0 0 1-16.971-7.029l-16-16C244.361 121.851 255.069 96 276.451 96H520c13.255 0 24 10.745 24 24v222.118l40.416-42.792c9.285-9.831 24.856-10.054 34.419-.492l10.823 10.823c9.372 9.372 9.372 24.569-.001 33.941zm-265.138 15.431A23.999 23.999 0 0 0 347.548 352H160V169.881l40.416 42.792c9.286 9.831 24.856 10.054 34.419.491l10.822-10.822c9.373-9.373 9.373-24.569 0-33.941L144.971 67.716c-9.373-9.373-24.569-9.373-33.941 0L10.343 168.402c-9.373 9.373-9.373 24.569 0 33.941l10.822 10.822c9.562 9.562 25.133 9.34 34.419-.491L96 169.881V392c0 13.255 10.745 24 24 24h243.549c21.382 0 32.09-25.851 16.971-40.971l-16.001-16z"></path>
      </svg>
      4
    </span>
</div>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
        None,
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt="">

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener" title="10:00">example.com – Oct 1</a>
  </header>

  <article class="onebox-body">
    <img src="https://example.com/av.png" class="thumbnail onebox-avatar" alt="">
<h4><a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener">A "title" &lt;with&gt; &amp; stuff</a></h4>
<div class="twitter-screen-name"><a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener">@user</a></div>

<div class="tweet">
    <span class="is-reply">
      <svg class="fa d-icon d-icon-reply svg-icon svg-string"><use href="#reply"></use></svg>
    </span>
  <span class="tweet-description">Hello <b>world</b> <a href="https://t.co/x" target="_blank" rel="nofollow ugc noopener">link</a> </span>
    <div class="quoted">
      <a class="quoted-link" href="https://x.com/q" rel="nofollow ugc noopener">
        <p class="quoted-title">Q <span>@qq</span></p>
      </a>

      <div>quoted</div>
    </div>
</div>

<div class="date">
  <a href="https://example.com/a?b=1&amp;c=2" class="timestamp" target="_blank" rel="nofollow ugc noopener">1:00 PM</a>

    <span class="like">
      <svg viewBox="0 0 512 512" width="14px" height="16px" aria-hidden="true"><path d="M462.3 62.6C407.5 15.9 326 24.3 275.7 76.2L256 96.5l-19.7-20.3C186.1 24.3 104.5 15.9 49.7 62.6c-62.8 53.6-66.1 149.8-9.9 207.9l193.5 199.8c12.5 12.9 32.8 12.9 45.3 0l193.5-199.8c56.3-58.1 53-154.3-9.8-207.9z"></path></svg>
      3
    </span>

    <span class="retweet">
      <svg viewBox="0 0 640 512" width="14px" height="16px" aria-hidden="true"><path d="M629.657 343.598L528.971 444.284c-9.373 9.372-24.568 9.372-33.941 0L394.343 343.598c-9.373-9.373-9.373-24.569 0-33.941l10.823-10.823c9.562-9.562 25.133-9.34 34.419.492L480 342.118V160H292.451a24.005 24.005 0 0 1-16.971-7.029l-16-16C244.361 121.851 255.069 96 276.451 96H520c13.255 0 24 10.745 24 24v222.118l40.416-42.792c9.285-9.831 24.856-10.054 34.419-.492l10.823 10.823c9.372 9.372 9.372 24.569-.001 33.941zm-265.138 15.431A23.999 23.999 0 0 0 347.548 352H160V169.881l40.416 42.792c9.286 9.831 24.856 10.054 34.419.491l10.822-10.822c9.373-9.373 9.373-24.569 0-33.941L144.971 67.716c-9.373-9.373-24.569-9.373-33.941 0L10.343 168.402c-9.373 9.373-9.373 24.569 0 33.941l10.822 10.822c9.562 9.562 25.133 9.34 34.419-.491L96 169.881V392c0 13.255 10.745 24 24 24h243.549c21.382 0 32.09-25.851 16.971-40.971l-16.001-16z"></path></svg>
      4
    </span>
</div>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
    );
}

#[test]
fn template_wikipedia() {
    check(
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt=""/>

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener" title="10:00">example.com &ndash; Oct 1</a>
  </header>

  <article class="onebox-body">
    <img src="https://example.com/img.png" class="thumbnail">

<h3><a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener">A &quot;title&quot; &lt;with&gt; &amp; stuff</a></h3>

<p>Some description here</p>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
        None,
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt="">

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener" title="10:00">example.com – Oct 1</a>
  </header>

  <article class="onebox-body">
    <img src="https://example.com/img.png" class="thumbnail">

<h3><a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener">A "title" &lt;with&gt; &amp; stuff</a></h3>

<p>Some description&nbsp;here</p>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
    );
}

#[test]
fn template_xkcd() {
    check(
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt=""/>

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener" title="10:00">example.com &ndash; Oct 1</a>
  </header>

  <article class="onebox-body">
    <h3><a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener">A &quot;title&quot; &lt;with&gt; &amp; stuff</a></h3>

  <div class="xkcd-image"><img src="https://example.com/img.png"></div>

<p>Some description here</p>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
        None,
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt="">

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener" title="10:00">example.com – Oct 1</a>
  </header>

  <article class="onebox-body">
    <h3><a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener">A "title" &lt;with&gt; &amp; stuff</a></h3>

  <div class="xkcd-image"><img src="https://example.com/img.png"></div>

<p>Some description&nbsp;here</p>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
    );
}

#[test]
fn template_stackexchange() {
    check(
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt=""/>

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener" title="10:00">example.com &ndash; Oct 1</a>
  </header>

  <article class="onebox-body">
    
<h4>
  <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener">A "title" <with> & stuff</a>
</h4>

<div class="tags">
  <strong></strong>
</div>

<div class="date">
  
  
  <a href="" target="_blank" rel="noopener">
    
  </a>
  on <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener"></a>
</div>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
        None,
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt="">

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener" title="10:00">example.com – Oct 1</a>
  </header>

  <article class="onebox-body">
    
<h4>
  <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener">A "title"  &amp; stuff</a>
</h4>

<div class="tags">
  <strong></strong>
</div>

<div class="date">
  
  
  <a href="" rel="noopener">
    
  </a>
  on <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener"></a>
</div>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
    );
}

#[test]
fn template_hackernews() {
    check(
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt=""/>

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener" title="10:00">example.com &ndash; Oct 1</a>
  </header>

  <article class="onebox-body">
    <h3><a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener">A &quot;title&quot; &lt;with&gt; &amp; stuff</a></h3>

  <p>Some description here</p>


<p>
  <span class="label1">d1 points</span> —
  <span class="label2">d2 comments</span> —
  <a href="https://news.ycombinator.com/user?id=Author" class="author" target="_blank" rel="noopener">Author</a> —
  <a href="https://example.com/a?b=1&amp;c=2" class="timestamp" target="_blank" rel="noopener">1:00 PM</a>
</p>


  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
        None,
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt="">

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener" title="10:00">example.com – Oct 1</a>
  </header>

  <article class="onebox-body">
    <h3><a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener">A "title" &lt;with&gt; &amp; stuff</a></h3>

  <p>Some description&nbsp;here</p>


<p>
  <span class="label1">d1 points</span> —
  <span class="label2">d2 comments</span> —
  <a href="https://news.ycombinator.com/user?id=Author" class="author" target="_blank" rel="nofollow ugc noopener">Author</a> —
  <a href="https://example.com/a?b=1&amp;c=2" class="timestamp" target="_blank" rel="nofollow ugc noopener">1:00 PM</a>
</p>


  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
    );
}

#[test]
fn template_instagram() {
    check(
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt=""/>

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener" title="10:00">example.com &ndash; Oct 1</a>
  </header>

  <article class="onebox-body">
    <h3><a href="https://example.com/a?b=1&c=2" target="_blank" rel="noopener">A &quot;title&quot; &lt;with&gt; &amp; stuff</a></h3>

  <div class="instagram-images">
    <a href="https://example.com/a?b=1&c=2" target="_blank" rel="noopener">
      <img class="instagram-image" src="https://example.com/img.png">
    </a>
  </div>

  <div class="instagram-description">Some description here</div>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
        None,
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt="">

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener" title="10:00">example.com – Oct 1</a>
  </header>

  <article class="onebox-body">
    <h3><a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener">A "title" &lt;with&gt; &amp; stuff</a></h3>

  <div class="instagram-images">
    <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener">
      <img class="instagram-image" src="https://example.com/img.png">
    </a>
  </div>

  <div class="instagram-description">Some description&nbsp;here</div>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
    );
}

#[test]
fn template_pdf() {
    check(
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt=""/>

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener" title="10:00">example.com &ndash; Oct 1</a>
  </header>

  <article class="onebox-body">
    <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener"><span class="pdf-onebox-logo"></span></a>

<h3><a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener">A &quot;title&quot; &lt;with&gt; &amp; stuff</a></h3>

  <p class="filesize">12 KB</p>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
        None,
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt="">

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener" title="10:00">example.com – Oct 1</a>
  </header>

  <article class="onebox-body">
    <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener"><span class="pdf-onebox-logo"></span></a>

<h3><a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener">A "title" &lt;with&gt; &amp; stuff</a></h3>

  <p class="filesize">12 KB</p>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
    );
}

#[test]
fn template_googleplayapp() {
    check(
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt=""/>

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener" title="10:00">example.com &ndash; Oct 1</a>
  </header>

  <article class="onebox-body">
    <h3>A &quot;title&quot; &lt;with&gt; &amp; stuff</h3>

<img src="https://example.com/img.png" class="thumbnail">
<p>Some description here</p>
<em>$12.00</em>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
        None,
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt="">

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener" title="10:00">example.com – Oct 1</a>
  </header>

  <article class="onebox-body">
    <h3>A "title" &lt;with&gt; &amp; stuff</h3>

<img src="https://example.com/img.png" class="thumbnail">
<p>Some description&nbsp;here</p>
<em>$12.00</em>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
    );
}

#[test]
fn template_pubmed() {
    check(
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt=""/>

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener" title="10:00">example.com &ndash; Oct 1</a>
  </header>

  <article class="onebox-body">
    <h4>
  <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener">A &quot;title&quot; &lt;with&gt; &amp; stuff</a>
</h4>

<div class="date">
  A, B,
  <i>J</i>, 2020-01-01
</div>

<p class="pubmed-abstract">
  Abstract text
</p>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
        None,
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt="">

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener" title="10:00">example.com – Oct 1</a>
  </header>

  <article class="onebox-body">
    <h4>
  <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener">A "title" &lt;with&gt; &amp; stuff</a>
</h4>

<div class="date">
  A, B,
  <i>J</i>, 2020-01-01
</div>

<p class="pubmed-abstract">
  Abstract text
</p>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
    );
}

#[test]
fn template_wikimedia() {
    check(
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt=""/>

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener" title="10:00">example.com &ndash; Oct 1</a>
  </header>

  <article class="onebox-body">
    <img src="https://example.com/t.png" class="thumbnail">

<h3><a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="noopener">A &quot;title&quot; &lt;with&gt; &amp; stuff</a></h3>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
        None,
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/a?b=1&amp;c=2">
  <header class="source">
      <img src="https://example.com/favicon.ico" class="site-icon" alt="">

      <a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener" title="10:00">example.com – Oct 1</a>
  </header>

  <article class="onebox-body">
    <img src="https://example.com/t.png" class="thumbnail">

<h3><a href="https://example.com/a?b=1&amp;c=2" target="_blank" rel="nofollow ugc noopener">A "title" &lt;with&gt; &amp; stuff</a></h3>

  </article>

  <div class="onebox-metadata">
    <span style="float: left;">Price: $5</span>
    <span style="float: right;">Rating: 5</span>
  </div>

  <div style="clear: both"></div>
</aside>
"####,
    );
}

#[test]
fn iframe_allowed() {
    check(
        r####"<iframe src="https://www.youtube.com/embed/x?a=1&b=2" width="480" height="270" frameborder="0" allowfullscreen onload="x()" data-original-href="https://youtu.be/x">fallback <b>text</b></iframe>"####,
        Some(&[
            r"\Ahttps://www\.youtube\.com/embed/",
            r"\Ahttps://player\.vimeo\.com/",
        ]),
        r####"<iframe src="https://www.youtube.com/embed/x?a=1&amp;b=2" width="480" height="270" frameborder="0" allowfullscreen="" data-original-href="https://youtu.be/x" seamless="seamless" sandbox="allow-same-origin allow-scripts allow-forms allow-popups allow-popups-to-escape-sandbox allow-presentation"></iframe>"####,
    );
}

#[test]
fn iframe_disallowed() {
    check(
        r####"<div class="o"><iframe src="https://evil.example/embed" width="100"></iframe></div>"####,
        Some(&[
            r"\Ahttps://www\.youtube\.com/embed/",
            r"\Ahttps://player\.vimeo\.com/",
        ]),
        r####"<div class="o"><iframe width="100" data-unsanitized-src="https://evil.example/embed" seamless="seamless" sandbox="allow-same-origin allow-scripts allow-forms allow-popups allow-popups-to-escape-sandbox allow-presentation"></iframe></div>"####,
    );
}

#[test]
fn iframe_without_src() {
    check(
        r####"<iframe width="10"></iframe>"####,
        Some(&[
            r"\Ahttps://www\.youtube\.com/embed/",
            r"\Ahttps://player\.vimeo\.com/",
        ]),
        r####"<iframe width="10" data-unsanitized-src="" seamless="seamless" sandbox="allow-same-origin allow-scripts allow-forms allow-popups allow-popups-to-escape-sandbox allow-presentation"></iframe>"####,
    );
}

#[test]
fn iframe_javascript_src() {
    check(
        r####"<iframe src="javascript:alert(1)"></iframe>"####,
        None,
        r####"<iframe seamless="seamless" sandbox="allow-same-origin allow-scripts allow-forms allow-popups allow-popups-to-escape-sandbox allow-presentation"></iframe>"####,
    );
}

#[test]
fn iframe_any_src_without_regexes() {
    check(
        r####"<iframe src="//cdn.example/x" scrolling="no"></iframe>"####,
        None,
        r####"<iframe src="//cdn.example/x" scrolling="no" seamless="seamless" sandbox="allow-same-origin allow-scripts allow-forms allow-popups allow-popups-to-escape-sandbox allow-presentation"></iframe>"####,
    );
}

#[test]
fn iframe_empty_regex_list() {
    check(
        r####"<iframe src="https://www.youtube.com/embed/x"></iframe>"####,
        Some(&[]),
        r####"<iframe data-unsanitized-src="https://www.youtube.com/embed/x" seamless="seamless" sandbox="allow-same-origin allow-scripts allow-forms allow-popups allow-popups-to-escape-sandbox allow-presentation"></iframe>"####,
    );
}

#[test]
fn styles() {
    check(
        r####"<div style="color: red; position: fixed; background: url(https://ex.com/a.png); --aspect-ratio: 16/9; width: expression(alert(1)); margin:0">x</div><span style="background-image: url('javascript:alert(1)')">y</span><p style="float:left;/* c */">z</p>"####,
        None,
        r####"<div style="color: red; position: fixed; background: url(https://ex.com/a.png); --aspect-ratio: 16/9;  margin:0">x</div><span>y</span><p style="float:left;/* c */">z</p>"####,
    );
}

#[test]
fn style_element() {
    check(
        r####"<style>.a { color: red; behavior: url(x.htc) } @import url(x.css); @media (max-width: 1px) { .b { top: 0 } }</style><p>t</p>"####,
        None,
        r####"<style>.a { color: red; }  @media (max-width: 1px) { .b { top: 0 } }</style><p>t</p>"####,
    );
}

#[test]
fn svg_children() {
    check(
        r####"<svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true" class="x" onload="y()"><title>t</title><g><path d="M0 0"/></g><path d="M1 1" fill-rule="evenodd" stroke="red"/><use href="#icon"/><use xlink:href="#other"/><circle r="1"/>text</svg>"####,
        None,
        r####"<svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true" class="x"><path d="M1 1" fill-rule="evenodd"></path><use href="#icon"></use><use xlink:href="#other"></use></svg>"####,
    );
}

#[test]
fn svg_use_absolute_href() {
    check(
        r####"<svg><use href="https://evil.example/s.svg#x"></use></svg>"####,
        None,
        r####"<svg><use></use></svg>"####,
    );
}

#[test]
fn data_attributes() {
    check(
        r####"<aside class="onebox" data-onebox-src="https://x.com" data-xml-bad="1"><div data-foo="1" data-bar-baz="2" data-="3" data-xmlx="4"><span data-id="7" data-Upper="8">s</span></div><p data-foo="no">p</p></aside>"####,
        None,
        r####"<aside class="onebox" data-onebox-src="https://x.com"><div data-foo="1" data-bar-baz="2"><span data-id="7" data-upper="8">s</span></div><p>p</p></aside>"####,
    );
}

#[test]
fn links() {
    check(
        r####"<a href="/relative" target="_blank">r</a> <a target="_blank">no href</a> <a href="//proto.example/x" target="_blank" rel="me">p</a> <a href="HTTPS://UP.example" target="_blank">u</a> <a href="mailto:a@b.c" target="_blank">m</a> <a href="javascript:alert(1)" target="_blank">j</a> <a href="ftp://f.example/" title="t" class="c" name="n x">f</a>"####,
        None,
        r####"<a href="/relative">r</a> <a href="#">no href</a> <a href="//proto.example/x" target="_blank" rel="nofollow ugc noopener">p</a> <a href="HTTPS://UP.example">u</a> <a href="mailto:a@b.c">m</a> <a>j</a> <a href="ftp://f.example/" title="t" class="c" name="n%20x" rel="nofollow ugc noopener">f</a>"####,
    );
}

#[test]
fn media() {
    check(
        r####"<video width="640" height="360" controls poster="https://ex.com/p.jpg" onplay="x()" autoplay muted loop playsinline controlslist="nodownload"><source src="https://ex.com/v.mp4" type="video/mp4"><source src="javascript:x" type="video/webm">Fallback</video><audio controls controlslist="nodownload"><source src="/a.mp3"></audio><embed src="https://ex.com/e.swf" type="application/x-shockwave-flash" width="1" height="1" allowscriptaccess="always">"####,
        None,
        r####"<video width="640" height="360" controls="" poster="https://ex.com/p.jpg" autoplay="" muted="" loop="" playsinline="" controlslist="nodownload"><source src="https://ex.com/v.mp4" type="video/mp4"><source type="video/webm">Fallback</video><audio controls="" controlslist="nodownload"><source src="/a.mp3"></audio><embed src="https://ex.com/e.swf" type="application/x-shockwave-flash" width="1" height="1">"####,
    );
}

#[test]
fn unsafe_elements() {
    check(
        r####"<form action="/x"><input name="a" value="b"><button>Go</button><select><option>1</option></select></form><script>alert(1)</script><noscript><b>ns</b></noscript><object data="x"></object><math><mi>x</mi></math>"####,
        None,
        r####"Go1"####,
    );
}

#[test]
fn details_table_img() {
    check(
        r####"<details><summary>Sum</summary><p>body</p></details><table width="1" onclick="x"><thead><tr><th scope="col">h</th></tr></thead><tbody><tr><td colspan="2">d</td></tr></tbody></table><img src="https://ex.com/i.png" srcset="https://ex.com/i2.png 2x" alt="a" width="1" height="2" loading="lazy" class="c">"####,
        None,
        r####"<details><summary>Sum</summary><p>body</p></details><table width="1"><thead><tr><th scope="col">h</th></tr></thead><tbody><tr><td colspan="2">d</td></tr></tbody></table><img src="https://ex.com/i.png" srcset="https://ex.com/i2.png 2x" alt="a" width="1" height="2" class="c">"####,
    );
}

#[test]
fn comments_and_whitespace() {
    check(
        r####"<div>a<!-- hidden --> b <br> c<hr><center>centered</center><font color="red">f</font></div>

<p> nbsp &amp; &lt;tag&gt; "quotes"</p>"####,
        None,
        r####"<div>a b <br> c<hr>centeredf</div>

<p>&nbsp;nbsp &amp; &lt;tag&gt; "quotes"</p>"####,
    );
}

#[test]
fn pre_newlines() {
    check(
        r####"<pre>

leading</pre><pre><code>
x</code></pre><textarea>
t</textarea>"####,
        None,
        r####"<pre>

leading</pre><pre><code>
x</code></pre>t"####,
    );
}

#[test]
fn onebox_aside() {
    check(
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/x">
  <header class="source">
    <img src="https://example.com/favicon.ico" class="site-icon" width="16" height="16">
    <a href="https://example.com/x" target="_blank" rel="nofollow ugc noopener">example.com</a>
  </header>
  <article class="onebox-body">
    <div class="aspect-image" style="--aspect-ratio:690/362;"><img src="https://example.com/i.png" class="thumbnail" width="690" height="362"></div>
    <h3><a href="https://example.com/x" target="_blank" rel="nofollow ugc noopener">Title</a></h3>
    <p>Desc</p>
  </article>
  <div style="clear: both"></div>
</aside>"####,
        None,
        r####"<aside class="onebox allowlistedgeneric" data-onebox-src="https://example.com/x">
  <header class="source">
    <img src="https://example.com/favicon.ico" class="site-icon" width="16" height="16">
    <a href="https://example.com/x" target="_blank" rel="nofollow ugc noopener">example.com</a>
  </header>
  <article class="onebox-body">
    <div class="aspect-image" style="--aspect-ratio:690/362;"><img src="https://example.com/i.png" class="thumbnail" width="690" height="362"></div>
    <h3><a href="https://example.com/x" target="_blank" rel="nofollow ugc noopener">Title</a></h3>
    <p>Desc</p>
  </article>
  <div style="clear: both"></div>
</aside>"####,
    );
}

#[test]
fn control_chars_and_entities() {
    check(
        r####"abc﷐d &#0; &#x1; &nbsp; &copy; <b title="&quot;">x</b>"####,
        None,
        r####"abcd �  &nbsp; © <b title="&quot;">x</b>"####,
    );
}

#[test]
fn headings_lists_quotes() {
    check(
        r####"<h1 id="h">H</h1><h4>4</h4><ul type="disc"><li value="2">a</li></ul><ol start="3" reversed><li>b</li></ol><blockquote cite="javascript:x">q</blockquote><q cite="https://ex.com">qq</q><abbr title="t">A</abbr><time datetime="2020" pubdate>t</time>"####,
        None,
        r####"<h1 id="h">H</h1><h4>4</h4><ul type="disc"><li value="2">a</li></ul><ol start="3" reversed=""><li>b</li></ol><blockquote>q</blockquote><q cite="https://ex.com">qq</q><abbr title="t">A</abbr><time datetime="2020" pubdate="">t</time>"####,
    );
}

#[test]
fn malformed() {
    check(
        r####"<div><p>unclosed <a href="https://ex.com">link <b>bold</div>tail<<script>x</script>"####,
        None,
        r####"<div><p>unclosed <a href="https://ex.com" rel="nofollow ugc noopener">link <b>bold</b></a></p></div><a href="https://ex.com" rel="nofollow ugc noopener"><b>tail&lt;</b></a>"####,
    );
}

#[test]
fn meta_and_head_in_fragment() {
    check(
        r####"<meta charset="iso-8859-1"><title>T</title><link rel="stylesheet" href="x.css"><base href="https://evil/">body"####,
        None,
        r####"<title>T</title>body"####,
    );
}
