;; Dockerfiles' stages, by their `FROM` lines' aliases (`AS builder`), or
;; else their images.  (Instructions are a flat list, so only the `FROM` line
;; is in its context.)

(((from_instruction as: (image_alias) @name) @container)
  (#set! structure.kind "stage"))

(((from_instruction (image_spec name: (image_name) @name) !as) @container)
  (#set! structure.kind "stage"))
