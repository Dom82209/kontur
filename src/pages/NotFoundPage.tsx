import { Compass } from "lucide-react";
import { Link } from "react-router-dom";
import { EmptyState } from "../components/ui/States";

export function NotFoundPage() {
  return <EmptyState icon={<Compass />} title="Экран не найден" text="Возможно, ссылка устарела." action={<Link className="btn" to="/">На главную</Link>} />;
}
