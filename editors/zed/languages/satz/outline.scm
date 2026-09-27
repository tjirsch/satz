(header
  name: (identifier) @name) @item

(header
  name: (string) @name) @item

(params "params" @name) @item

(use_statement
  "use" @context
  path: (string) @name) @item

(use_interface
  "use" @context
  "interface" @context
  names: (_) @name) @item

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

(offers
  "offers" @context
  path: (string) @name) @item

(private
  "private" @context
  resource: (identifier) @name) @item

(request
  "request" @context
  param: (identifier) @name) @item

(interface
  "interface" @context
  name: (string) @name) @item

(export
  "export" @context
  name: (string) @name) @item

(hcl_block "hcl" @name) @item

(each
  "each" @context
  list: (identifier) @name
  "by" @context
  key: (identifier) @context) @item

(block
  key: (_) @name
  name: (_)? @context) @item
