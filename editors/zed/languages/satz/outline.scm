(header
  name: (identifier) @name) @item

(params "params" @name) @item

(use_statement
  "use" @context
  path: (string) @name) @item

(suppress
  "suppress" @context
  type: (identifier) @context
  name: (string) @name) @item

(claim
  "claim" @context
  framework: (string) @context
  version: (string) @context
  control: (string) @name
  coverage: (coverage) @context) @item

(question
  "question" @context
  name: (identifier) @name) @item

(action
  "action" @context
  name: (string) @name) @item

(notice
  "notice" @context
  name: (identifier) @name) @item

(export
  "export" @context
  name: (string) @name) @item

(hcl_block "hcl" @name) @item

(block
  key: (_) @name
  name: (_)? @context) @item
