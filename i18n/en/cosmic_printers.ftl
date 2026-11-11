tooltip-printers = Printers
job-count =
    { $count ->
        [one] { $count } print job
       *[other] { $count } print jobs
    }
printer-warning = { $printer }: { $message }
unknown = Unknown
printers = Printers
show-icon = Show icon
show-always = Always
show-printers = When printers exist
show-jobs = When printing
send-to-front = Send to the front
cancel-job = Cancel job
cancel-all = Cancel all jobs
show-properties = Show properties
out-of-paper = Out of paper
paper-jam = Paper jam
low-on-paper = Low on paper
out-of-toner = Out of toner or ink
low-toner = Low toner or ink
cartridge-missing = Ink cartridge missing
cover-open = Cover open
offline = Offline
connecting = Connecting...
missing-filter = Missing print filter
paused = Paused
printer-error = Printer error
stopped = Stopped
not-accepting = Not accepting jobs
no-cups-client = CUPS client tools are not installed.
list-failed = Could not list printers.
cancel-one-title = Cancel print job
cancel-one-body = Do you want to cancel printing of '{ $name }'?
cancel-all-title = Cancel all jobs
cancel-all-body = Do you really want to cancel all jobs?
cancel-printing = Cancel printing
keep = Keep
