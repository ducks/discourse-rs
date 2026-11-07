// The reference's tests/unit/lib/to-markdown-test.js, its bodies run as
// written in the editor page (rich-editor-helper.js inPage) with every
// default extension: HTML into markdown (static/js/rich-editor/
// to-markdown.js), as a paste or a quoted selection is. Its table and image
// cases are in rich-editor-table.spec.js and rich-editor-media.spec.js.
const { test } = require("./helpers");
const { qunitTest } = require("./rich-editor-helper");

const qtest = (name, body) => qunitTest(test, name, body);

test.describe("rich editor to-markdown", () => {
  qtest("converts styles between normal words", async function (assert, { toMarkdown, v, codeBlock }) {
    const html = `Line with <s>styles</s> <b><i>between</i></b> words.`;
    const markdown = `Line with ~~styles~~ ***between*** words.`;
    assert.strictEqual(await toMarkdown(html), markdown);

    assert.strictEqual(
      await toMarkdown("A <b>bold </b>word"),
      "A **bold** word"
    );
    assert.strictEqual(
      await toMarkdown("A <b>bold</b>, word"),
      "A **bold**, word"
    );
  });

  qtest("converts inline nested styles", async function (assert, { toMarkdown, v, codeBlock }) {
    let html = `<em>Italicised line with <strong>some random</strong> <b>bold</b> words.</em>`;
    let markdown = `*Italicised line with **some random** **bold** words.*`;
    assert.strictEqual(await toMarkdown(html), markdown);

    html = `<i class="fa">Italicised line
     with <b title="strong">some<br>
     random</b> <s>bold</s> words.</i>`;
    markdown = `*Italicised line with **some\nrandom** ~~bold~~ words.*`;
    assert.strictEqual(await toMarkdown(html), markdown);

    // eslint-disable-next-line no-irregular-whitespace
    html = `<span>this is<span> </span></span><strong>bold</strong><span><span> </span>statement</span>`;
    // Non-breaking spaces may be preserved by ProseMirror
    const result = await toMarkdown(html);
    assert.true(result.includes("**bold**"), "bold formatting preserved");
  });

  qtest("only treats bold font weights as strong", async function (assert, { toMarkdown, v, codeBlock }) {
    assert.strictEqual(
      await toMarkdown(
        `some <span style="font-weight: 500;">medium</span> text`
      ),
      "some medium text"
    );

    assert.strictEqual(
      await toMarkdown(`some <span style="font-weight: 700;">bold</span> text`),
      "some **bold** text"
    );

    assert.strictEqual(
      await toMarkdown(
        `some <span style="font-weight: 1000;">black</span> text`
      ),
      "some **black** text"
    );

    assert.strictEqual(
      await toMarkdown(
        `some <span style="font-weight: 650.5;">variable</span> text`
      ),
      "some **variable** text"
    );

    assert.strictEqual(
      await toMarkdown(
        `some <span style="font-weight: bold;">bold</span> text`
      ),
      "some **bold** text"
    );
  });

  qtest("converts a link", async function (assert, { toMarkdown, v, codeBlock }) {
    let html = `<a href="https://discourse.org">Discourse</a>`;
    let markdown = `[Discourse](https://discourse.org)`;
    assert.strictEqual(await toMarkdown(html), markdown);

    html = `<a href="https://discourse.org">Disc\n\n\nour\n\nse</a>`;
    markdown = `[Disc our se](https://discourse.org)`;
    assert.strictEqual(await toMarkdown(html), markdown);
  });

  qtest("converts a link which is an attachment", async function (assert, { toMarkdown, v, codeBlock }) {
    let html = `<a class="attachment" href="https://discourse.org/pdfs/stuff.pdf">stuff.pdf</a>`;
    let markdown = `[stuff.pdf|attachment](https://discourse.org/pdfs/stuff.pdf)`;
    assert.strictEqual(await toMarkdown(html), markdown);
  });

  qtest("put raw URL instead of converting the link", async function (assert, { toMarkdown, v, codeBlock }) {
    let url = "https://discourse.org";
    const html = () => `<a href="${url}">${url}</a>`;

    assert.strictEqual(await toMarkdown(html()), url);

    url = "discourse.org/t/topic-slug/1";
    assert.strictEqual(await toMarkdown(html()), url);
  });

  qtest("skip empty link", async function (assert, { toMarkdown, v, codeBlock }) {
    assert.strictEqual(
      await toMarkdown(`<a href="https://example.com"></a>`),
      ""
    );
  });

  qtest("converts heading tags", async function (assert, { toMarkdown, v, codeBlock }) {
    const html = `
    <h1>Heading 1</h1>
    <h2>Heading 2</h2>

    \t  <h3>Heading 3</h3>


    <h4>Heading 4</h4>



  <h5>Heading 5</h5>




  <h6>Heading 6</h6>
    `;
    const markdown = `# Heading 1\n\n## Heading 2\n\n### Heading 3\n\n#### Heading 4\n\n##### Heading 5\n\n###### Heading 6`;
    assert.strictEqual(await toMarkdown(html), markdown);
  });

  qtest("converts ul list tag", async function (assert, { toMarkdown, v, codeBlock }) {
    let html = `
    <ul>
      <li>Item 1</li>
      <li>
        Item 2
        <ul>
          <li>Sub Item 1</li>
          <li>Sub Item 2</li>
          <li>Sub Item 3<ul><li>Sub <i>Sub</i> Item 1</li><li>Sub <b>Sub</b> Item 2</li></ul></li>
        </ul>
      </li>
      <li>Item 3</li>
    </ul>
    `;
    let markdown = `* Item 1\n* Item 2\n  * Sub Item 1\n  * Sub Item 2\n  * Sub Item 3\n    * Sub *Sub* Item 1\n    * Sub **Sub** Item 2\n* Item 3`;
    assert.strictEqual(await toMarkdown(html), markdown);

    html = `
  <ul>
    <li><span>Bullets at level 1</span></li>
    <li><span>Bullets at level 1</span></li>  <ul>    <li><span>Bullets at level 2</span></li>    <li><span>Bullets at level 2</span></li>    <ul>      <li><span>Bullets at level 3</span></li>    </ul>    <li><span>Bullets at level 2</span></li>  </ul>  <li><span>Bullets at level 1</span></li></ul>  `;
    markdown = `* Bullets at level 1
* Bullets at level 1
  * Bullets at level 2
  * Bullets at level 2
    * Bullets at level 3
  * Bullets at level 2
* Bullets at level 1`;
    assert.strictEqual(await toMarkdown(html), markdown);
  });

  qtest("stripes unwanted inline tags", async function (assert, { toMarkdown, v, codeBlock }) {
    const html = `
    <p>Lorem ipsum <span>dolor sit amet, consectetur</span> <strike>elit.</strike></p>
    <p>Ut minim veniam, <label>quis nostrud</label> laboris <nisi> ut aliquip ex ea</nisi> commodo.</p>
    `;
    const markdown = `Lorem ipsum dolor sit amet, consectetur ~~elit.~~\n\nUt minim veniam, quis nostrud laboris ut aliquip ex ea commodo.`;
    assert.strictEqual(await toMarkdown(html), markdown);
  });

  qtest("supporting html tags by keeping them", async function (assert, { toMarkdown, v, codeBlock }) {
    // <del> is now properly converted to ~~ markdown strikethrough
    let html =
      "Lorem <del>ipsum dolor</del> sit <big>amet, <ins>consectetur</ins></big>";
    let output =
      "Lorem ~~ipsum dolor~~ sit <big>amet, <ins>consectetur</ins></big>";
    assert.strictEqual(await toMarkdown(html), output);

    html = `Lorem <del style="font-weight: bold">ipsum dolor</del> sit <big>amet, <ins onclick="alert('hello')">consectetur</ins></big>`;
    output = `Lorem **~~ipsum dolor~~** sit <big>amet, <ins>consectetur</ins></big>`;
    assert.strictEqual(await toMarkdown(html), output);

    html = `<a href="http://example.com" onload="">Lorem <del style="font-weight: bold">ipsum dolor</del> sit</a>.`;
    output = `[Lorem **~~ipsum dolor~~** sit](http://example.com).`;
    assert.strictEqual(await toMarkdown(html), output);

    html = `Lorem <del>ipsum dolor</del> sit.`;
    output = `Lorem ~~ipsum dolor~~ sit.`;
    assert.strictEqual(await toMarkdown(html), output);

    html = `Have you tried clicking the <kbd>Help Me!</kbd> button?`;
    assert.strictEqual(await toMarkdown(html), html);

    html = `<mark>This is highlighted!</mark>`;
    assert.strictEqual(await toMarkdown(html), html);

    // Strikethrough inside a link serializes with the marks in different order
    // due to how ProseMirror handles nested marks
    html = `Lorem <a href="http://example.com"><del>ipsum dolor</del> sit.</a>`;
    output = `Lorem ~~[ipsum dolor](http://example.com)~~[ sit.](http://example.com)`;
    assert.strictEqual(await toMarkdown(html), output);
  });

  qtest("converts code tags", async function (assert, { toMarkdown, v, codeBlock }) {
    let html = `Lorem ipsum dolor sit amet,
  <pre><code>var helloWorld = () => {
  alert('    hello \t\t world    ');
    return;
}
helloWorld();</code></pre>
  consectetur.`;
    let output = `Lorem ipsum dolor sit amet, \n\n\`\`\`\nvar helloWorld = () => {\n  alert('    hello \t\t world    ');\n    return;\n}\nhelloWorld();\n\`\`\`\n\n consectetur.`;

    assert.strictEqual(await toMarkdown(html), output);

    html = `Lorem ipsum dolor sit amet, <code>var helloWorld = () => {
  alert('    hello \t\t world    ');
    return;
}
helloWorld();</code>consectetur.`;
    output = `Lorem ipsum dolor sit amet, \`var helloWorld = () => { alert(' hello world '); return; } helloWorld();\`consectetur.`;

    assert.strictEqual(await toMarkdown(html), output);
  });

  qtest("keeps highlighted code lines in a single code block", async function (assert, { toMarkdown, v, codeBlock }) {
    const html = `<p>Before <code>id</code>.</p><pre class="language-bash"><code><div class="token-line"><span class="token plain">website </span><span class="token comment"># Root directory</span><br></div><div class="token-line"><span>   └── docs</span><br></div><div class="token-line"><span></span><br></div><div class="token-line"><span>      └── hello.md</span><br></div></code></pre><p>After.</p>`;

    assert.strictEqual(
      await toMarkdown(html),
      "Before `id`.\n\n```bash\nwebsite # Root directory\n   └── docs\n\n      └── hello.md\n```\n\nAfter.",
      "preserves code boundaries, indentation, blank lines, and the language"
    );
  });

  qtest("reads the code language from a wrapper around the pre", async function (assert, { toMarkdown, v, codeBlock }) {
    const html = `<div class="language-ruby"><pre><code><div>puts :hi<br></div></code></pre></div>`;

    assert.strictEqual(
      await toMarkdown(html),
      "```ruby\nputs :hi\n```",
      "a language class on the container is used when the pre has none"
    );
  });

  qtest("reads the code language from the code element", async function (assert, { toMarkdown, v, codeBlock }) {
    const html = `<pre><code class="lang-ruby">puts :hi</code></pre>`;

    assert.strictEqual(
      await toMarkdown(html),
      "```ruby\nputs :hi\n```",
      "the short class prefix is recognized"
    );
  });

  qtest("ignores the autodetect language marker", async function (assert, { toMarkdown, v, codeBlock }) {
    const html = `<pre><code class="lang-auto">puts :hi</code></pre>`;

    assert.strictEqual(
      await toMarkdown(html),
      "```\nputs :hi\n```",
      "autodetection is not a language"
    );
  });

  qtest("keeps an explicit language over a class", async function (assert, { toMarkdown, v, codeBlock }) {
    const html = `<pre data-params="ruby" class="language-bash"><code><div>puts :hi<br></div></code></pre>`;

    assert.strictEqual(
      await toMarkdown(html),
      "```ruby\nputs :hi\n```",
      "data-params wins over a language class"
    );
  });

  qtest("preserves explicit breaks in code blocks", async function (assert, { toMarkdown, v, codeBlock }) {
    assert.strictEqual(
      await toMarkdown("<pre><code>first<br><br>  last</code></pre>"),
      "```\nfirst\n\n  last\n```",
      "explicit breaks remain newlines"
    );
  });

  qtest("leaves other preformatted structures unchanged", async function (assert, { toMarkdown, v, codeBlock }) {
    const { normalizeCodeBlocks } = codeBlock;
    const doc = new DOMParser().parseFromString(
      "<pre><code>first\n  last</code></pre><pre><code><div>first</div><div>last</div></code></pre><pre><code>first<div>last<br></div></code></pre><div>outside<br></div>",
      "text/html"
    );
    const original = doc.body.innerHTML;

    normalizeCodeBlocks(doc);

    assert.strictEqual(
      doc.body.innerHTML,
      original,
      "only normalizes code composed entirely of div lines with explicit breaks"
    );
  });

  qtest("converts blockquote tag", async function (assert, { toMarkdown, v, codeBlock }) {
    let html = "<blockquote>Lorem ipsum</blockquote>";
    let output = "> Lorem ipsum";
    assert.strictEqual(await toMarkdown(html), output);

    html =
      "<blockquote>Lorem ipsum</blockquote><blockquote><p>dolor sit amet</p></blockquote>";
    output = "> Lorem ipsum\n\n> dolor sit amet";
    assert.strictEqual(await toMarkdown(html), output);

    html =
      "<blockquote>\nLorem ipsum\n<blockquote><p>dolor <blockquote>sit</blockquote> amet</p></blockquote></blockquote>";
    output = "> Lorem ipsum\n>\n> > dolor\n> >\n> > > sit\n> >\n> >  amet";
    assert.strictEqual(await toMarkdown(html), output);
  });

  qtest("converts ol list tag", async function (assert, { toMarkdown, v, codeBlock }) {
    const html = `Testing
    <ol>
      <li>Item 1</li>
      <li>
        Item 2
        <ol start="100">
          <li>Sub Item 1</li>
          <li>Sub Item 2</li>
        </ol>
      </li>
      <li>Item 3</li>
    </ol>
    `;
    const markdown = `Testing\n\n1. Item 1\n2. Item 2\n   100. Sub Item 1\n   101. Sub Item 2\n3. Item 3`;
    assert.strictEqual(await toMarkdown(html), markdown);
  });

  qtest("converts loose lists (with paragraphs inside items)", async function (assert, { toMarkdown, v, codeBlock }) {
    const html = `<ul><li><p>Item 1</p></li><li><p>Item 2</p></li></ul>`;
    assert.strictEqual(await toMarkdown(html), `* Item 1\n\n* Item 2`);

    const olHtml = `<ol><li><p>First</p></li><li><p>Second</p></li></ol>`;
    assert.strictEqual(await toMarkdown(olHtml), `1. First\n\n2. Second`);
  });

  qtest("converts ol with custom start attribute", async function (assert, { toMarkdown, v, codeBlock }) {
    const html = `<ol start="5"><li>Fifth</li><li>Sixth</li></ol>`;
    const markdown = `5. Fifth\n6. Sixth`;
    assert.strictEqual(await toMarkdown(html), markdown);
  });

  qtest("converts nested ol with custom start attributes", async function (assert, { toMarkdown, v, codeBlock }) {
    const html = `
    <ol start="3">
      <li>Third
        <ol start="10">
          <li>Tenth</li>
          <li>Eleventh</li>
        </ol>
      </li>
      <li>Fourth</li>
    </ol>
    `;
    const markdown = `3. Third\n   10. Tenth\n   11. Eleventh\n4. Fourth`;
    assert.strictEqual(await toMarkdown(html), markdown);
  });

  qtest("converts list tag from word", async function (assert, { toMarkdown, v, codeBlock }) {
    const html = `Sample<!--StartFragment-->
    <p class=MsoListParagraphCxSpFirst style='text-indent:-.25in;mso-list:l0 level1 lfo1'>
      <![if !supportLists]>
      <span style='font-family:Symbol;mso-fareast-font-family:Symbol;mso-bidi-font-family:  Symbol;mso-bidi-font-weight:bold'>
        <span style='mso-list:Ignore'>·
          <span style='font:7.0pt "Times New Roman"'> </span>
        </span>
      </span>
      <![endif]>
      <b>Item 1
        <o:p></o:p>
      </b>
    </p>
    <p class=MsoListParagraphCxSpMiddle style='text-indent:-.25in;mso-list:l0 level2 lfo1'>
      <![if !supportLists]>
      <span style='font-family:Symbol;mso-fareast-font-family:Symbol;mso-bidi-font-family:  Symbol;mso-bidi-font-style:italic'>
        <span style='mso-list:Ignore'>·
          <span style='font:7.0pt "Times New Roman"'> </span>
        </span>
      </span>
      <![endif]>
      <i>Item 2
        <o:p></o:p>
      </i>
    </p>
    <p class=MsoListParagraphCxSpMiddle style='text-indent:-.25in;mso-list:l0 level3 lfo1'>
      <![if !supportLists]>
      <span style='font-family:Symbol;mso-fareast-font-family:Symbol;mso-bidi-font-family:  Symbol'>
        <span style='mso-list:Ignore'>·
          <span style='font:7.0pt "Times New Roman"'> </span>
        </span>
      </span>
      <![endif]>Item 3 </p>
    <p class=MsoListParagraphCxSpLast style='text-indent:-.25in;mso-list:l0 level1 lfo1'>
      <![if !supportLists]>
      <span style='font-family:Symbol;mso-fareast-font-family:Symbol;mso-bidi-font-family:  Symbol'>
        <span style='mso-list:Ignore'>·
          <span style='font:7.0pt "Times New Roman"'> </span>
        </span>
      </span>
      <![endif]>Item 4</p>
    <!--EndFragment-->List`;
    const markdown = `Sample\n\n* **Item 1**\n  * *Item 2*\n    * Item 3\n* Item 4\n\nList`;
    assert.strictEqual(await toMarkdown(html), markdown);
  });

  qtest("converts quote style from word", async function (assert, { toMarkdown, v, codeBlock }) {
    const html = `Intro<!--StartFragment-->
    <p class=MsoQuote style='margin-left:.5in'>First quoted line</p>
    <p class=MsoQuote style='margin-left:.5in'>Second quoted line</p>
    <!--EndFragment-->Outro`;
    const markdown = `Intro\n\n> First quoted line\n>\n> Second quoted line\n\nOutro`;
    assert.strictEqual(await toMarkdown(html), markdown);
  });

  qtest("strips word comments and tracked deletions", async function (assert, { toMarkdown, v, codeBlock }) {
    const html = `<!--StartFragment-->
    <p class=MsoNormal>Identify significant accounts<a class=msocomanchor
      href="#_msocom_1" style='mso-comment-reference:SK_1;mso-comment-date:1'><span
      class=MsoCommentReference>[J1]</span></a> and relevant assertions.</p>
    <p class=MsoNormal>This involves <del>old removed phrase </del>using risk procedures.</p>
    <div style='mso-element:comment-list'><div style='mso-element:comment'>
      <p class=MsoCommentText><a name="_msocom_1"></a>Consider removing.</p>
    </div></div>
    <!--EndFragment-->`;

    const markdown = await toMarkdown(html);

    assert.true(markdown.includes("Identify significant accounts and"));
    assert.true(markdown.includes("This involves using risk procedures."));
    assert.false(markdown.includes("[J1]"));
    assert.false(markdown.includes("Consider removing."));
    assert.false(markdown.includes("old removed phrase"));
  });

  qtest("drops the lang spans a publishing tool wraps each sentence in", async function (assert, { toMarkdown, v, codeBlock }) {
    const html = `<p><span class="sentence" lang="en">So I have heard.</span> <span class="sentence" lang="en">At one time.</span></p>`;

    assert.strictEqual(
      await toMarkdown(html),
      "So I have heard. At one time.",
      "removes generated lang spans"
    );
  });

  qtest("keeps the space a dropped lang span held at its edge", async function (assert, { toMarkdown, v, codeBlock }) {
    const html = `<p><span class="sentence" lang="en">So I have heard. </span><span class="sentence" lang="en">At one time.</span></p>`;

    assert.strictEqual(
      await toMarkdown(html),
      "So I have heard. At one time.",
      "sentences stay separated"
    );
  });

  qtest("drops an authored lang span too, arriving as HTML (accepted)", async function (assert, { toMarkdown, v, codeBlock }) {
    const html = `<p>He said <span lang="ja">日本語</span> loudly.</p>`;

    assert.strictEqual(
      await toMarkdown(html),
      "He said 日本語 loudly.",
      "only markdown carries lang into the composer"
    );
  });

  qtest("keeps the markup a dropped lang span wrapped", async function (assert, { toMarkdown, v, codeBlock }) {
    const html = `<p><span lang="en">A <b>bold</b> <mark>marked</mark> <ruby lang="ja">漢<rt>かん</rt></ruby> line.</span></p>`;

    assert.strictEqual(
      await toMarkdown(html),
      'A **bold** <mark>marked</mark> <ruby lang="ja">漢<rt>かん</rt></ruby> line.',
      "preserves nested markup"
    );
  });

  qtest("drops the document-language spans Word for the web stamps on runs", async function (assert, { toMarkdown, v, codeBlock }) {
    const html = `<div class="OutlineElement"><p class="Paragraph">
      <span class="TextRun" lang="EN-GB"><span class="NormalTextRun" lang="EN-GB">Note:</span></span>
      <span class="TextRun" lang="EN-GB"><span class="NormalTextRun" lang="EN-GB"> materiality</span></span>
    </p></div>`;

    const markdown = await toMarkdown(html);

    assert.strictEqual(markdown, "Note: materiality");
    assert.false(markdown.includes("<span"));
    assert.false(markdown.includes("lang="));
  });

  qtest("cleans a real Word for the web run (lang on TextRun, EOP marker)", async function (assert, { toMarkdown, v, codeBlock }) {
    // Trimmed from a real Word for the web clipboard.
    const html = `<span data-contrast="auto" xml:lang="PT-BR" lang="PT-BR" class="TextRun SCXW1 BCX0"><span class="NormalTextRun SCXW1 BCX0">This</span><span class="NormalTextRun SCXW1 BCX0"><span> </span></span><span class="NormalTextRun SCXW1 BCX0">is something</span></span><span class="EOP SCXW1 BCX0" data-ccp-props="{}"> </span>`;

    const markdown = await toMarkdown(html);

    assert.strictEqual(markdown, "This is something");
    assert.false(markdown.includes("<span"));
    assert.false(markdown.includes("lang="));
  });

  qtest("keeps mention/hash class", async function (assert, { toMarkdown, v, codeBlock }) {
    const html = `
      <p>User mention: <a class="mention" href="/u/discourse">@discourse</a></p>
      <p>Group mention: <a class="mention-group" href="/groups/discourse">@discourse-group</a></p>
      <p>Category link: <a class="hashtag" href="/c/foo/1">#<span>foo</span></a></p>
      <p>Sub-category link: <a class="hashtag" href="/c/foo/bar/2">#<span>foo:bar</span></a></p>
    `;

    const markdown = `User mention: @discourse\n\nGroup mention: @discourse-group\n\nCategory link: #foo\n\nSub-category link: #foo:bar`;

    assert.strictEqual(await toMarkdown(html), markdown);
  });

  qtest("strips user status from mentions", async function (assert, { toMarkdown, v, codeBlock }) {
    const statusHtml = `
        <img class="emoji user-status"
             src="/images/emoji/twitter/desert_island.png?v=${v}"
             title="vacation">
    `;
    const html = `Mentioning <a class="mention" href="/u/andrei">@andrei${statusHtml}</a>`;
    const expectedMarkdown = `Mentioning @andrei`;

    assert.strictEqual(await toMarkdown(html), expectedMarkdown);
  });

  qtest("keeps hashtag-cooked and converts to bare hashtag with type", async function (assert, { toMarkdown, v, codeBlock }) {
    const html = `
      <p dir="ltr">This is <a class="hashtag-cooked" href="/c/ux/14" data-type="category" data-slug="ux">
      <svg class="fa d-icon d-icon-folder svg-icon svg-node">
        <use href="#folder"></use>
      </svg>
      <span>ux</span>
      </a> and <a class="hashtag-cooked" href="/tag/design" data-slug="design">
      <svg class="fa d-icon d-icon-tag svg-icon svg-node">
        <use href="#tag"></use>
      </svg>
      <span>design</span>
      </a> and <a class="hashtag-cooked" href="/c/uncategorized/design/22" data-type="category" data-slug="design" data-ref="uncategorized:design">
      <svg class="fa d-icon d-icon-folder svg-icon svg-node">
        <use href="#folder"></use>
      </svg>
      <span>design</span>
      </a></p>
    `;

    const markdown = `This is #ux::category and #design and #uncategorized:design`;
    assert.strictEqual(await toMarkdown(html), markdown);
  });

  qtest("keeps emoji and ignores click count", async function (assert, { toMarkdown, v, codeBlock }) {
    const html = `
      <p>
        A <a href="http://example.com" data-clicks="1" aria-label="link clicked 1 time">link</a> with click count
        and <img class="emoji" title=":boom:" src="https://d11a6trkgmumsb.cloudfront.net/images/emoji/twitter/boom.png?v=5" alt=":boom:" /> emoji.
      </p>
    `;

    const markdown = `A [link](http://example.com) with click count and :boom: emoji.`;

    assert.strictEqual(await toMarkdown(html), markdown);
  });

  qtest("keeps emoji syntax for custom emoji", async function (assert, { toMarkdown, v, codeBlock }) {
    const html = `
      <p>
        <img class="emoji emoji-custom" title=":custom_emoji:" src="https://d11a6trkgmumsb.cloudfront.net/images/emoji/custom_emoji" alt=":custom_emoji:" />
      </p>
    `;

    const markdown = `:custom_emoji:`;

    assert.strictEqual(await toMarkdown(html), markdown);
  });

  qtest("restores clipboard non-breaking spaces around inline nodes", async function (assert, { toMarkdown, v, codeBlock }) {
    // The copied nbsp beside the emoji must become a regular space or it won't
    // cook; an author-typed nbsp (in "to keep") is left alone.
    const html = `<span style="color: rgb(0,0,0)">use the<span>&nbsp;</span></span><img class="emoji" title=":heart:" alt=":heart:" /><span style="color: rgb(0,0,0)"><span>&nbsp;</span>to&nbsp;keep</span>`;

    assert.strictEqual(await toMarkdown(html), `use the :heart: to\u00a0keep`);
  });

  qtest("converts quotes to markdown", async function (assert, { toMarkdown, v, codeBlock }) {
    let html = `
    <p>there is a quote below</p>
    <aside class="quote no-group" data-username="foo" data-post="1" data-topic="2">
    <div class="title" style="cursor: pointer;">
    <div class="quote-controls"><span class="svg-icon-title" title="expand/collapse"><svg class="fa d-icon d-icon-chevron-down svg-icon svg-string" xmlns="http://www.w3.org/2000/svg"><use href="#chevron-down"></use></svg></span><a href="/t/hello-world-i-am-posting-an-image/158/1" title="go to the quoted post" class="back"><svg class="fa d-icon d-icon-arrow-up svg-icon svg-string" xmlns="http://www.w3.org/2000/svg"><use href="#arrow-up"></use></svg></a></div>
    <img alt="" width="20" height="20" src="" class="avatar"> foo:</div>
    <blockquote>
    <p>this is a quote</p>
    </blockquote>
    </aside>
    <p>there is a quote above</p>
    `;

    let markdown = `
there is a quote below

[quote="foo, post:1, topic:2"]
this is a quote

[/quote]

there is a quote above
`;

    assert.strictEqual(await toMarkdown(html), markdown.trim());

    html = `
    <aside class="quote no-group" data-username="foo" data-post="1" data-topic="2" data-full="false">
    <blockquote>
    <p>not a full quote</p>
    </blockquote>
    </aside>
    `;

    markdown = `
[quote="foo, post:1, topic:2"]
not a full quote

[/quote]
`;

    assert.strictEqual(await toMarkdown(html), markdown.trim());

    html = `
    <aside class="quote no-group" data-username="foo" data-post="1" data-topic="2" data-full="true">
    <blockquote>
    <p>full quote</p>
    </blockquote>
    </aside>
    `;

    markdown = `
[quote="foo, post:1, topic:2, full:true"]
full quote

[/quote]
`;

    assert.strictEqual(await toMarkdown(html), markdown.trim());
  });

  qtest("converts nested quotes to markdown", async function (assert, { toMarkdown, v, codeBlock }) {
    let html = `
      <aside class="quote no-group">
        <blockquote>
          <aside class="quote no-group">
            <blockquote>
              <p dir="ltr">test</p>
            </blockquote>
          </aside>
          <p dir="ltr">test2</p>
        </blockquote>
      </aside>
    `;

    let markdown = `
[quote]
[quote]
test

[/quote]

test2

[/quote]
`;

    assert.strictEqual(await toMarkdown(html), markdown.trim());
  });
});
