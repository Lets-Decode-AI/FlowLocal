import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import Row from '../components/ui/Row';
import Group from '../components/ui/Group';
import Segmented from '../components/ui/Segmented';
import Switch from '../components/ui/Switch';
import { keycapsFor } from '../components/ui/Keycap';
import Menu from '../components/ui/Menu';

afterEach(cleanup);

describe('ui primitives', () => {
  it('switch_toggles_and_has_role', () => {
    const onChange = vi.fn();
    render(<Switch checked={false} onChange={onChange} label="Launch at login" />);
    const sw = screen.getByRole('switch', { name: 'Launch at login' });
    expect(sw.getAttribute('aria-checked')).toBe('false');
    fireEvent.click(sw);
    expect(onChange).toHaveBeenCalledWith(true);
  });

  it('segmented_selects_option', () => {
    const onChange = vi.fn();
    render(<Segmented options={[{ value: 'a', label: 'Installed' }, { value: 'b', label: 'Catalog' }]} value="a" onChange={onChange} />);
    expect(screen.getByRole('tablist')).toBeTruthy();
    expect(screen.getByRole('tab', { name: 'Installed' }).getAttribute('aria-selected')).toBe('true');
    fireEvent.click(screen.getByRole('tab', { name: 'Catalog' }));
    expect(onChange).toHaveBeenCalledWith('b');
  });

  it('segmented_arrow_keys_move_selection', () => {
    const onChange = vi.fn();
    render(<Segmented options={[{ value: 'a', label: 'One' }, { value: 'b', label: 'Two' }, { value: 'c', label: 'Three' }]} value="a" onChange={onChange} />);
    const one = screen.getByRole('tab', { name: 'One' });
    expect(one.tabIndex).toBe(0);
    expect(screen.getByRole('tab', { name: 'Two' }).tabIndex).toBe(-1);
    fireEvent.keyDown(one, { key: 'ArrowRight' });
    expect(onChange).toHaveBeenLastCalledWith('b');
    expect(document.activeElement).toBe(screen.getByRole('tab', { name: 'Two' }));
    fireEvent.keyDown(one, { key: 'ArrowLeft' });
    expect(onChange).toHaveBeenLastCalledWith('c');
    fireEvent.keyDown(one, { key: 'End' });
    expect(onChange).toHaveBeenLastCalledWith('c');
    fireEvent.keyDown(one, { key: 'Home' });
    expect(onChange).toHaveBeenLastCalledWith('a');
  });

  it('keycaps_render_mac_glyphs', () => {
    const { container, unmount } = render(<>{keycapsFor({ keys: ['ControlLeft', 'AltRight', 'ShiftLeft', 'MetaLeft', 'Fn'] }, 'mac')}</>);
    expect(Array.from(container.querySelectorAll('kbd')).map((k) => k.textContent)).toEqual(['⌃', '⌥', '⇧', '⌘', 'fn']);
    unmount();
    const other = render(<>{keycapsFor({ keys: ['ControlLeft', 'AltRight', 'ShiftLeft', 'MetaLeft'] }, 'other')}</>);
    expect(Array.from(other.container.querySelectorAll('kbd')).map((k) => k.textContent)).toEqual(['Ctrl', 'Alt', 'Shift', 'Win']);
  });

  it('row_renders_detail_and_accessory', () => {
    render(<Group title="General"><Row label="Show Flow Bar" detail="Floating pill while dictating" accessory={<span>acc</span>} /></Group>);
    expect(screen.getByText('General')).toBeTruthy();
    expect(screen.getByText('Show Flow Bar')).toBeTruthy();
    expect(screen.getByText('Floating pill while dictating')).toBeTruthy();
    expect(screen.getByText('acc')).toBeTruthy();
  });

  it('menu_closes_on_escape_and_outside_click_and_returns_focus', () => {
    const onSelect = vi.fn();
    render(<div><Menu label="Options" items={[{ label: 'Use', onSelect }, { label: 'Delete', onSelect: () => {}, destructive: true }]} /><button>Elsewhere</button></div>);
    const trigger = screen.getByRole('button', { name: 'Options' });
    fireEvent.click(trigger);
    expect(screen.getAllByRole('menuitem')).toHaveLength(2);
    fireEvent.keyDown(screen.getByRole('menuitem', { name: 'Use' }), { key: 'ArrowDown' });
    expect(document.activeElement).toBe(screen.getByRole('menuitem', { name: 'Delete' }));
    fireEvent.keyDown(document.activeElement as Element, { key: 'Escape' });
    expect(screen.queryByRole('menu')).toBeNull();
    expect(document.activeElement).toBe(trigger);
    fireEvent.click(trigger);
    fireEvent.mouseDown(screen.getByRole('button', { name: 'Elsewhere' }));
    expect(screen.queryByRole('menu')).toBeNull();
    fireEvent.click(trigger);
    fireEvent.keyDown(screen.getByRole('menuitem', { name: 'Use' }), { key: 'Tab' });
    expect(screen.queryByRole('menu')).toBeNull();
    fireEvent.click(trigger);
    fireEvent.click(screen.getByRole('menuitem', { name: 'Use' }));
    expect(onSelect).toHaveBeenCalled();
    expect(document.activeElement).toBe(trigger);
  });
});
