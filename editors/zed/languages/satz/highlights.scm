; Generic patterns first, specific ones after: for one node Zed keeps the
; last matching capture, so a quoted key ends up @property, not @string.

(comment) @comment

(number) @number
(boolean) @boolean
(reference) @variable
(string) @string
(hcl_string) @string
(escape_sequence) @string.escape

["{" "}" "[" "]"] @punctuation.bracket
["=" ","] @punctuation.delimiter

[
  "estate" "pack" "version"
  "params"
  "use" "as" "when"
  "claim"
  "question" "oneof"
  "action"
  "notice"
  "export" "description"
  "interface"
  "suppress" "role"
  "hcl" "trust"
] @keyword
(coverage) @keyword

; Keys and names. A key that names a provider type is a type; the schema,
; not the syntax, decides for the rest, so they are properties.
(attribute key: (_) @property)
(block key: (_) @property)
(block name: (_) @label)
((block key: (identifier) @type)
  (#match? @type "^google_"))

(header name: (identifier) @title)
(param name: (identifier) @variable)
(question name: (identifier) @variable)
(notice name: (identifier) @variable)
(export name: (string) @variable)
(interface name: (string) @title)
(use_statement type: (identifier) @type)
(use_statement condition: (identifier) @variable)
(suppress type: (identifier) @type)

; {name} inside a string: the parameter it interpolates.
(interpolation
  "{" @punctuation.special
  (parameter) @variable.special
  "}" @punctuation.special)

; The raw passthrough, when no HCL grammar is installed to take it over.
(hcl_content) @embedded
