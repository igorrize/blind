Review the change against the task and the code rules.
Read all files in `changed` in one turn; use `grep` to see how things are used elsewhere.
Return one short finding per real problem in `findings`: what is wrong and where. No style wishes beyond the rules.
Set `approved` to true only if the change does what the task asks for and breaks no rule.
