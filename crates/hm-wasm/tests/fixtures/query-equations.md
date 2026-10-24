These two small MIT fixtures were authored with the current `hm-model`, then
encoded by `hm-hwp5::write_new` and `hm-hwpx::write_new`. They contain the same
three equations: a body script with quotes, backticks, Korean, a backslash,
a newline and an emoji; an empty script in a table cell; and `α + β` in a
text box inside that cell. The surrounding body and cell text include an emoji
to exercise UTF-16 wire offsets. They are synthetic query fixtures.
