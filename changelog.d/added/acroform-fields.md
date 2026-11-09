HTML form controls with a PDF-standard equivalent are now emitted as fillable
AcroForm fields instead of only a static drawing: text/search/tel/url/email/
password/number and date/time inputs and `<textarea>` become text fields
(date/time fields carry the usual `AFDate_*`/`AFTime_*` format actions),
checkboxes and radio groups become button fields (one field per `name`),
`<select>` becomes a combo or list box (`<optgroup>` labels are kept as
display prefixes, never as selectable options), and buttons become
pushbuttons (submit/reset get `SubmitForm`/`ResetForm` actions when the
form names a target). Controls with no PDF-standard equivalent
(`hidden`, `file`, `color`, `range`) keep the static rendering.
`--enable-forms` is now accepted (it is the default) and `--disable-forms`
suppresses field emission. Works in streaming mode too.
