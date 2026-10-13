#!/usr/bin/env python3
"""Apply the pinned clap-wrapper AU view initialization fix, idempotently."""
from pathlib import Path
import sys

path = Path(sys.argv[1]) / 'src/detail/auv2/wrappedview.asinclude.mm'
source = path.read_text()
marker = '// Reveler-rs: initialize NSView before publishing resize callbacks.'
if marker not in source:
    before = '  ui = *cont;\n  canary = 0xbeebbeeb;'
    after = f'''  {marker}
  self = [super initWithFrame:NSZeroRect];
  if (!self) return nil;

  ui = *cont;
  canary = 0xbeebbeeb;'''
    late_init = '  self = [super initWithFrame:NSMakeRect(0, 0, size.width, size.height)];'
    assert source.count(before) == 1 and source.count(late_init) == 1, 'Unexpected clap-wrapper source'
    source = source.replace(before, after).replace(late_init, '  [self setFrame:NSMakeRect(0, 0, size.width, size.height)];')
    path.write_text(source)
