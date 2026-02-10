Protocol version 7.3 uses netstrings with hardcoded fields. Replace this with JSON objects that use meaningful field names.
Where possible, the JSON field names should match the POSIX or Linux kernel names. After writing the new code, stop and ask for
feedback on the code before writing any tests.

Additional considerations:
* For the /read function, return the data in application/octet-stream form.