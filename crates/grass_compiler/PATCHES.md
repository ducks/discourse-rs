# grass_compiler 0.13.4, patched

A copy of grass_compiler 0.13.4 from crates.io (MIT, LICENSE), patched so
that theme SCSS compiles to what dart-sass 1.104.0 prints: the version the
reference's sass-embedded gem runs, through Stylesheet::Compiler. The
workspace's `[patch.crates-io]` builds `grass` against it; the workspace
excludes it, so it is not formatted or linted with the port.

Each patch is marked `discourse-rs:` in the source and has a test in
src/stylesheet/scss.rs (`grass_patches`), its expected output what dart-sass
1.104.0 prints for the same input. src/stylesheet/mod.rs's `compile_tests`
compile what Rails fed dart-sass for the color definitions and Horizon's
stylesheets and compare the output byte for byte.

To compare with upstream:

    diff -ru ~/.cargo/registry/src/index.crates.io-*/grass_compiler-0.13.4 crates/grass_compiler

## Imports

`evaluate/visitor.rs` (`find_import`, `import_like_node`): import-only
files (`_lib.import.scss`) are found for `@import` alone, never for `@use`
or `@forward`, as dart-sass's importer does; grass looked for them for
every rule. A URL with an explicit extension (`@import "other.scss"`) is
looked up in the load paths too, not only next to the importing file
(dart-sass's `_tryPath` for each load path). Discourse's core stylesheets
have import-only files (lib/viewport, lib/breakpoints, lib/container) that
Horizon's stylesheets `@use`. Test: `import_only_files_are_for_import_alone`.

## A declaration after a nested rule

`evaluate/visitor.rs` (`copy_parent_after_sibling`): dart-sass's
`_copyParentAfterSibling`. A declaration (or comment) that follows a nested
rule goes into a copy of its parent rule placed after that nested rule, so
`.a{b:1;.c{d:2}e:3}` prints `.a{b:1}.a .c{d:2}.a{e:3}`; grass merged `e:3`
into the first `.a`. Test: `a_declaration_after_a_nested_rule_keeps_its_place`.

## Empty custom properties

`parse/stylesheet.rs` and `evaluate/visitor.rs`: a custom property may be
empty (`--x:;`), and is written (`ast/css.rs`, `is_invisible`), as
dart-sass parses and prints it; grass refused to parse it, and dropped a
blank one. Test: `empty_custom_properties_are_written`.

## Parentheses after a backtracked parse

`parse/value.rs` (`parse_paren_expr`): the in-parentheses flag is restored
however the parenthesized expression's parse ends, as dart-sass's `finally`
restores it. A nested selector such as `a:not(.q)` is first tried as a
declaration and backtracked; the flag it left set made every later `1/2` a
division. Test: `a_backtracked_selector_leaves_division_alone`.

## Values written by toCssString

`value/mod.rs` (`to_css_string`): dart-sass's `Value.toCssString`, which
interpolation, string concatenation and `@warn` use, always serializes in
the expanded style. grass used the output style, so `#{$list}` lost its
`, ` and a computed color its long form in compressed output (Discourse's
color definitions are all interpolated). Test:
`interpolation_writes_values_expanded`.

## Compressed media queries and @supports

`serializer.rs`: compressed output leaves out the space after `@media` when
the first query starts with a condition, joins conditions with `and ` (no
space before), and leaves out the space after `@supports` before a
parenthesized condition, as dart-sass's `visitCssMediaRule`,
`_visitMediaQuery` and `visitCssSupportsRule` do. Test:
`compressed_media_queries_and_supports_conditions`.

## Numbers

`serializer.rs` (`write_float`, `write_rounded`): dart-sass's `_writeNumber`
and `_writeRounded`. An integer is written as such (fuzzily so only under
inspect), a number short enough is written directly (compressed output
drops the leading zero of `0.5`, though not of `-0.5`), any other is
rounded to ten decimals on its shortest decimal form, half up by the next
digit, and `-0` stays `-0`. grass rounded the binary value with
`format!("{:.10}")` and dropped every leading zero. Test:
`numbers_are_written_as_dart_sass_writes_them`.

## Colors

`color/mod.rs`, `serializer.rs`, `builtin/functions/color/{rgb,other}.rs`:
legacy colors follow dart-sass 1.104's color spaces module. Channels are
never rounded along the way: `rgb()` clamps without rounding, conversions
between RGB and HSL use the channels as computed (dart-sass's
`SrgbColorSpace` and `HslColorSpace`), `mix()`, `invert()`, `scale-color()`,
`adjust-color()` and `change-color()` work on them as they are. A color
keeps the space it was made in: `hsl()` makes an HSL color, and
`lighten()`, `darken()`, `saturate()`, `desaturate()`, `adjust-hue()`,
`complement()` and the HSL branch of the update functions work in HSL and
convert back to the color's own space (`changeHsl`, `toSpace`);
`rgba($color, $alpha)` converts to RGB first. Only `red()`, `green()` and
`blue()` round, as dart-sass's do.

Writing a color is `_writeLegacyColor`: compressed, the shortest of a name,
a short or long hex code, `rgb()` or `hsl()` (non-integer channels as
percentages); expanded, an HSL color as `hsl()`, a color written in the
source as written (`rgb()` when it came from `rgb()`), else a name, a long
hex code or `rgb()`; an out-of-gamut color as `hsl()`. grass rounded every
channel to an integer and wrote hex or `rgba()` with integer channels.
Test: `colors_keep_their_channels_and_space`.

## Identifiers in calculations

`parse/value.rs`, `evaluate/visitor.rs`: an identifier of its own in a
calculation is an unquoted string written as is (`calc(alpha + 0.1)`, from
relative color syntax), or one of the constants `pi`, `e`, `infinity`,
`-infinity` and `NaN`, as dart-sass 1.104 evaluates calculations; grass
refused with `Expected "(" or "."`. Test:
`calculations_take_identifiers_and_constants`.

## Lints

`lib.rs`: `unexpected_cfgs` and `mismatched_lifetime_syntaxes` allowed, so
the copy builds quietly on the toolchain the port uses.
