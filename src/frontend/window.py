from PySide6 import QtCore, QtGui, QtWidgets

from frontend.application_grid import ApplicationsPanel
from frontend.buttons import (
    BUTTONS,
    application_action,
    applications_input_action,
    configure_badges,
)
from frontend.settings import INITIAL_WINDOW_SIZE, WINDOW_STYLESHEET
from frontend.widgets import GlassIconButton, LiquidGlassFrame, SearchBox, add_shadow

from backend.apps.application_search import ApplicationData

class MainWindow(QtWidgets.QWidget):
    def __init__(
        self,
        applications=None,
        badges=None,
    ):
        super().__init__()
        self._use_configured_badges = badges is None
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
        main_layout.setAlignment(QtCore.Qt.AlignTop)

        self.glass_panel = LiquidGlassFrame(radius=22)
        add_shadow(self.glass_panel)

        panel_layout = QtWidgets.QVBoxLayout(self.glass_panel)
        panel_layout.setContentsMargins(0, 0, 0, 0)
        panel_layout.setSpacing(0)

        search_area = QtWidgets.QWidget()
        search_area.setObjectName("searchArea")
        search_layout = QtWidgets.QHBoxLayout(search_area)
        search_layout.setContentsMargins(12, 6, 8, 6)
        search_layout.setSpacing(12)

        self.search_box = SearchBox()
        self.search_box.input.textChanged.connect(
            self._handle_applications_input
        )
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
        self.applications_panel.content_height_changed.connect(
            self._resize_visible_applications
        )
        self.applications_panel.hide()
        panel_layout.addWidget(self.applications_panel)
        main_layout.addWidget(self.glass_panel)

        self.set_applications(applications)
        if badges is not None:
            self.set_badges(badges)

        self.search_box.input.setFocus()

    @QtCore.Slot(str)
    def _handle_applications_input(self, user_input: str):
        applications_input_action(
            self,
            user_input,
        )

    @QtCore.Slot()
    def toggle_visibility(self):
        if self.isVisible():
            self.hide()
            return

        apps_obj = ApplicationData()
        apps = apps_obj.get_apps()
        self.set_applications(apps)

        self.show()
        self.raise_()
        self.activateWindow()
        self.search_box.input.setFocus()

    def set_applications(self, applications):
        """Сохраняет Applications и обновляет сетку, если она уже открыта."""
        self.applications = applications
        self.applications_panel.set_applications(applications)
        if self._use_configured_badges:
            self.set_badges(configure_badges(applications))
        if self.applications_panel.isVisible():
            self._resize_for_applications()
        elif not self.isVisible():
            self._prepare_main_window()

    def set_badges(self, badges):
        """Устанавливает плашки из словаря ``{name: applications}``."""
        self.badges = self.applications_panel.set_badges(badges)
        return self.badges

    def show_applications(self):
        """Показывает по одной плитке для каждого элемента Applications.apps."""
        self.divider.show()
        self.applications_panel.show()
        self._resize_for_applications()

    def showEvent(self, event):
        super().showEvent(event)
        QtCore.QTimer.singleShot(50, self._finish_show)

    def hideEvent(self, event):
        self.reset_to_main_window()
        super().hideEvent(event)

    def _resize_for_applications(self):
        window_height = self._expanded_window_height()
        self.setFixedSize(INITIAL_WINDOW_SIZE[0], window_height)
        self.glass_panel.setFixedHeight(window_height - 12)

    @QtCore.Slot()
    def _resize_visible_applications(self):
        if self.applications_panel.isVisible():
            self._resize_for_applications()

    def _expanded_window_height(self):
        panel_height = self.applications_panel.preferred_height()
        return INITIAL_WINDOW_SIZE[1] + 8 + panel_height

    def _prepare_main_window(self):
        self.glass_panel.setFixedHeight(INITIAL_WINDOW_SIZE[1] - 12)
        self.setFixedSize(INITIAL_WINDOW_SIZE[0], self._expanded_window_height())

    def _finish_show(self):
        if not self.isVisible():
            return

        self.center_on_screen()
        if not self.applications_panel.isVisible():
            self.setFixedSize(*INITIAL_WINDOW_SIZE)

    @QtCore.Slot()
    def center_on_screen(self):
        screen = self.screen() or QtWidgets.QApplication.primaryScreen()
        if screen is None:
            return

        centered_geometry = self.frameGeometry()
        centered_geometry.moveCenter(screen.availableGeometry().center())
        self.move(centered_geometry.topLeft())

    def reset_to_main_window(self):
        self.applications_panel.hide()
        self.divider.hide()
        self._prepare_main_window()
