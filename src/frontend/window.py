from PySide6 import QtCore, QtGui, QtWidgets

from frontend.application_grid import ApplicationsPanel
from frontend.buttons import BUTTONS, application_action
from frontend.settings import INITIAL_WINDOW_SIZE, WINDOW_STYLESHEET
from frontend.widgets import GlassIconButton, LiquidGlassFrame, SearchBox, add_shadow


class MainWindow(QtWidgets.QWidget):
    def __init__(self, applications=None):
        super().__init__()
        self.setWindowFlags(
            QtCore.Qt.FramelessWindowHint
            | QtCore.Qt.WindowStaysOnTopHint
            | QtCore.Qt.Tool
            | QtCore.Qt.X11BypassWindowManagerHint
        )
        self.setAttribute(QtCore.Qt.WA_TranslucentBackground)
        self.setFixedSize(*INITIAL_WINDOW_SIZE)
        self.setStyleSheet(WINDOW_STYLESHEET)

        self.close_shortcut = QtGui.QShortcut(
            QtGui.QKeySequence(QtCore.Qt.Key_Escape),
            self,
        )
        self.close_shortcut.activated.connect(self.hide)

        main_layout = QtWidgets.QVBoxLayout(self)
        main_layout.setContentsMargins(6, 6, 6, 6)
        main_layout.setSpacing(0)

        glass_panel = LiquidGlassFrame(radius=22)
        add_shadow(glass_panel)

        panel_layout = QtWidgets.QVBoxLayout(glass_panel)
        panel_layout.setContentsMargins(0, 0, 0, 0)
        panel_layout.setSpacing(0)

        search_area = QtWidgets.QWidget()
        search_area.setObjectName("searchArea")
        search_layout = QtWidgets.QHBoxLayout(search_area)
        search_layout.setContentsMargins(12, 6, 8, 6)
        search_layout.setSpacing(12)

        self.search_box = SearchBox()
        search_layout.addWidget(self.search_box, 1)

        buttons_layout = QtWidgets.QHBoxLayout()
        buttons_layout.setSpacing(6)
        buttons_layout.setContentsMargins(0, 0, 0, 0)

        self.buttons = {}
        for config in BUTTONS:
            button = GlassIconButton(
                config.icon,
                config.tooltip,
                config.size,
                active=config.name == "button1",
            )
            button.clicked.connect(
                lambda checked=False, action=config.action: action(self)
            )

            self.buttons[config.name] = button
            setattr(self, config.name, button)
            buttons_layout.addWidget(button)

        search_layout.addLayout(buttons_layout)
        panel_layout.addWidget(search_area)

        self.divider = QtWidgets.QFrame()
        self.divider.setObjectName("windowDivider")
        self.divider.setFixedHeight(1)
        self.divider.hide()
        panel_layout.addWidget(self.divider)

        self.applications_panel = ApplicationsPanel()
        self.applications_panel.application_selected.connect(
            lambda application: application_action(self, application)
        )
        self.applications_panel.hide()
        panel_layout.addWidget(self.applications_panel)
        main_layout.addWidget(glass_panel)

        self.set_applications(applications)

        self.search_box.input.setFocus()

    @QtCore.Slot()
    def toggle_visibility(self):
        if self.isVisible():
            self.hide()
            return

        self.show()
        self.raise_()
        self.activateWindow()
        self.search_box.input.setFocus()

    def set_applications(self, applications):
        """Сохраняет Applications и обновляет сетку, если она уже открыта."""
        self.applications = applications
        self.applications_panel.set_applications(applications)
        if self.applications_panel.isVisible():
            self._resize_for_applications()

    def show_applications(self):
        """Показывает по одной плитке для каждого элемента Applications.apps."""
        self.divider.show()
        self.applications_panel.show()
        self._resize_for_applications()

    def _resize_for_applications(self):
        panel_height = self.applications_panel.preferred_height()
        window_height = INITIAL_WINDOW_SIZE[1] + 8 + panel_height
        self.setFixedSize(INITIAL_WINDOW_SIZE[0], window_height)

    @QtCore.Slot()
    def center_on_screen(self):
        screen = self.screen() or QtWidgets.QApplication.primaryScreen()
        if screen is None:
            return

        centered_geometry = self.frameGeometry()
        centered_geometry.moveCenter(screen.availableGeometry().center())
        self.move(centered_geometry.topLeft())
