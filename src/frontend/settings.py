# Общие настройки окна. Размеры указываются как (ширина, высота).
INITIAL_WINDOW_SIZE = (520, 72)
APPLICATION_COLUMNS = 5
APPLICATION_TILE_SIZE = (84, 82)
APPLICATION_MAX_VISIBLE_ROWS = 4

GLASS_ICON_COLOR = "#f4f4f5"


WINDOW_STYLESHEET = """
    QWidget {
        color: #f4f4f5;
        font-family: "SF Pro Display", "Inter", "Roboto", sans-serif;
        font-size: 13px;
        background: transparent;
    }

    QLineEdit {
        color: #f4f4f5;
        background: transparent;
        border: none;
        selection-background-color: rgba(255, 255, 255, 45);
        font-size: 17px;
        font-weight: 500;
    }

    QLineEdit::placeholder {
        color: rgba(244, 244, 245, 115);
    }

    QFrame#windowDivider {
        background: rgba(255, 255, 255, 18);
        border: none;
    }

    QPushButton#glassButton {
        background: rgba(255, 255, 255, 10);
        border: 1px solid rgba(255, 255, 255, 18);
        border-radius: 13px;
        padding: 0;
    }

    QPushButton#glassButton:hover {
        background: rgba(255, 255, 255, 20);
        border-color: rgba(255, 255, 255, 25);
    }

    QPushButton#glassButton:pressed {
        background: rgba(255, 255, 255, 28);
    }

    QPushButton#glassButton[active="true"] {
        background: rgba(255, 255, 255, 31);
        border-color: rgba(255, 255, 255, 26);
    }

    QPushButton#glassButton[active="true"]:hover {
        background: rgba(255, 255, 255, 38);
    }

    QPushButton#badgeButton {
        color: rgba(244, 244, 245, 205);
        background: rgba(255, 255, 255, 10);
        border: 1px solid rgba(255, 255, 255, 20);
        border-radius: 9px;
        padding: 0 11px;
        font-size: 11px;
        font-weight: 500;
    }

    QPushButton#badgeButton:hover {
        color: #f4f4f5;
        background: rgba(255, 255, 255, 20);
        border-color: rgba(255, 255, 255, 28);
    }

    QPushButton#badgeButton:pressed,
    QPushButton#badgeButton[active="true"] {
        color: #ffffff;
        background: rgba(125, 211, 252, 35);
        border-color: rgba(165, 243, 252, 80);
    }

    QToolButton#applicationTile {
        color: #ededee;
        background: rgba(255, 255, 255, 9);
        border: 1px solid rgba(255, 255, 255, 16);
        border-radius: 12px;
        padding: 5px 3px;
        font-size: 10px;
    }

    QToolButton#applicationTile:hover {
        background: rgba(255, 255, 255, 20);
        border-color: rgba(255, 255, 255, 24);
    }

    QLabel#emptyApplications {
        color: rgba(244, 244, 245, 110);
        font-size: 13px;
    }

    QScrollArea {
        background: transparent;
        border: none;
    }

    QScrollBar:vertical {
        background: transparent;
        width: 5px;
        margin: 4px 0;
    }

    QScrollBar::handle:vertical {
        background: rgba(255, 255, 255, 42);
        border-radius: 2px;
        min-height: 24px;
    }

    QScrollBar::add-line:vertical,
    QScrollBar::sub-line:vertical {
        height: 0;
    }
"""
