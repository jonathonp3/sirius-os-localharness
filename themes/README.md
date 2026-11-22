# Themes

`solarized-light-golden-sand.xml` is the GtkSourceView scheme used as the
source for the web UI's **Golden Sand** palette. The browser cannot read a host
XML file, so the palette is embedded in `src/index.html` (`themes['golden-sand']`)
and drives both the UI variables and the highlight.js token colors.

Keep the XML and the embedded palette in sync when changing colors.
