# 假的新楓之谷視窗：標題含 MapleStory（程式用標題判斷是不是遊戲），記錄收到的按鍵
import sys
import tkinter as tk

log = open(sys.argv[1], "a", encoding="utf-8")
root = tk.Tk()
root.title("MapleStory")
root.geometry("980x700+10+10")
root.configure(bg="#2d6a4f")
tk.Label(root, text="GAME (MapleStory)", font=("Arial", 36), bg="#2d6a4f", fg="white").pack(expand=True)


def on_key(e):
    log.write("key:" + e.keysym + "\n")
    log.flush()


root.bind("<Key>", on_key)
root.mainloop()
