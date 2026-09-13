; The text between the braces of a top-level `hcl { … }` is HCL. Zed hands it
; to the HCL grammar when the terraform extension is installed; otherwise it
; stays Satz's own embedded text.
((hcl_block body: (hcl_body (hcl_content) @injection.content))
  (#set! injection.language "hcl"))
