"""Disposable window used as a binding target in tests."""
import sys
import tkinter as tk

title = "PeekGuard Test Target"
lifetime = int(sys.argv[1]) if len(sys.argv) > 1 else 60

root = tk.Tk()
root.title(title)
root.geometry("520x360+220+180")
tk.Label(root, text="peekguard test target", font=("Segoe UI", 15)).pack(expand=True)
root.after(lifetime * 1000, root.destroy)
root.mainloop()
