import { motion } from "motion/react";
import { NavLink, useLocation } from "react-router-dom";
import { NAV, weekPath } from "../../app/nav";
import { bridge } from "../../lib/bridge";
import { useBridgeQuery } from "../../lib/hooks/useBridgeQuery";

export function Sidebar() {
  const { pathname } = useLocation();
  const inbox = useBridgeQuery(() => bridge.getInboxFiles(), [], ["inbox:file-added", "operation:completed"]);
  const subjects = useBridgeQuery(() => bridge.getSubjects(), []);
  const inboxCount = inbox.data?.length ?? 0;

  const isActive = (to: string) => (to === "/" ? pathname === "/" : to === "/week" ? pathname.startsWith("/subjects/") || pathname === "/week" : pathname.startsWith(to));

  return (
    <nav className="sidebar" aria-label="Основная навигация">
      <div className="sidebar__brand">
        <span className="brand-mark" aria-hidden />
        <span>Контур</span>
      </div>
      <ul className="sidebar__nav">
        {NAV.map((item) => {
          const active = isActive(item.to);
          const Icon = item.icon;
          return (
            <li key={item.to}>
              <NavLink to={item.to} className="navitem" data-active={active} aria-current={active ? "page" : undefined}>
                {active && <motion.span layoutId="nav-active" className="navitem__bg" transition={{ type: "spring", stiffness: 500, damping: 40 }} />}
                <Icon className="navitem__icon" />
                <span className="navitem__label">{item.label}</span>
                {item.to === "/inbox" && inboxCount > 0 && <span className="navitem__count num">{inboxCount}</span>}
              </NavLink>
            </li>
          );
        })}
      </ul>
      <div className="sidebar__section eyebrow">Предметы</div>
      <ul className="sidebar__subjects scroll">
        {subjects.data?.map((s) => {
          const active = pathname.startsWith(`/subjects/${s.id}/`);
          return (
            <li key={s.id}>
              <NavLink to={weekPath(s.id, s.currentWeek)} className="subjitem" data-active={active} title={s.name}>
                <span className="subjitem__tick" aria-hidden />
                <span className="subjitem__name">{s.name}</span>
                <span className="subjitem__week num">{String(s.currentWeek).padStart(2, "0")}</span>
              </NavLink>
            </li>
          );
        })}
      </ul>
    </nav>
  );
}
