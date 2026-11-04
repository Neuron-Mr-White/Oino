#!/usr/bin/env python3
"""[owner-fix-2] Real X input for the rig (XTEST on the rig's Xvfb): wheel clicks and pointer
drags go through the X server -> GTK -> WebKitGTK exactly like a mouse, which WebDriver wheel
actions on WebKitWebDriver do not (they were dropped or sign-flipped in the rig).

  DISPLAY=:195 xin.py move X Y | wheel up|down N [delay_ms] | down | up | drag X Y X2 Y2 [steps]
Several commands can be chained with ';' in one argument list: xin.py move 10 10 ';' wheel up 3
"""
import sys, time
from Xlib import X, display
from Xlib.ext import xtest

d = display.Display()

def move(x, y):
    xtest.fake_input(d, X.MotionNotify, x=int(x), y=int(y)); d.sync()

def button(b, press):
    xtest.fake_input(d, X.ButtonPress if press else X.ButtonRelease, int(b)); d.sync()

def run(cmd):
    if not cmd:
        return
    op, *a = cmd
    if op == "move":
        move(a[0], a[1])
    elif op == "wheel":
        b = 4 if a[0] == "up" else 5
        delay = float(a[2]) / 1000 if len(a) > 2 else 0.016
        for _ in range(int(a[1])):
            button(b, True); button(b, False); time.sleep(delay)
    elif op == "down":
        button(1, True)
    elif op == "up":
        button(1, False)
    elif op == "drag":
        x, y, x2, y2 = map(int, a[:4]); steps = int(a[4]) if len(a) > 4 else 20
        move(x, y); time.sleep(0.05); button(1, True); time.sleep(0.05)
        for i in range(1, steps + 1):
            move(x + (x2 - x) * i / steps, y + (y2 - y) * i / steps); time.sleep(0.03)
        time.sleep(0.05); button(1, False)
    elif op == "sleep":
        time.sleep(float(a[0]) / 1000)
    else:
        sys.exit(f"unknown op {op}")

cmd = []
for tok in sys.argv[1:] + [";"]:
    if tok == ";":
        run(cmd); cmd = []
    else:
        cmd.append(tok)
